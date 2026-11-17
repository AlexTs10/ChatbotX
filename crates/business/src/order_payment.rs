//! `packages/business/src/order-payment/{service,settle}.ts`: the provider
//! page an order is paid on, reading it back (return, webhook, expiry scan),
//! refunds, and the payment side of a cancel or delete.
//!
//! TS holds the order row lock (`FOR UPDATE`) around the provider call. Here
//! the SQLite writer closures are sync, so a per-order [`LockMap`] lock is
//! held instead, the provider is called outside the write transaction, and
//! the write checks the rows again (`markPaid` only moves an unpaid,
//! not-cancelled order; status updates name their allowed `from`).

use std::sync::LazyLock;

use chatbotx_core::encryption::{Encryption, SHOP_TOKEN_TTL_MS, ShopCustomerClaims};
use chatbotx_core::js::encode_uri_component;
use chatbotx_core::utils::web_url::join_app_url;
use chatbotx_db::repo::models::{Order, OrderPayment};
use chatbotx_db::repo::{self, find_many, find_one, new_id, now_ms, order as order_repo};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_kv::lock::{LockGuard, LockMap, LockOpts};

use crate::errors::{BusinessError, Result};
use crate::order::{self, Actor, Deps, is_awaiting_online_payment, is_online};
use crate::payment_provider::{self, not_connected, to_exception};
use crate::payments::{CheckoutStatus, CreateCheckout, Gateway, PResult, PaymentMetadata};
use crate::tx;

/// Stripe closes a Checkout Session no sooner than 30 minutes after it opens.
const MIN_PROVIDER_PAGE_MS: i64 = 31 * 60_000;
/// An open page is offered again only with this long left on it.
const REUSE_MARGIN_MS: i64 = 2 * 60_000;
/// A payment still settling at the deadline is looked at again this much later.
const PROCESSING_RECHECK_MS: i64 = 60 * 60_000;
/// A provider that did not answer at the deadline is asked again this much later.
const RETRY_LATER_MS: i64 = 5 * 60_000;
/// An open attempt is swept this long after its page's end.
const STALE_ATTEMPT_GRACE_MS: i64 = 15 * 60_000;
/// The statuses in which an order still holds the money and the stock.
const OPEN_ORDER_STATUSES: &[&str] = &["pending", "accepted", "shipped"];

const LATE_PAYMENT_CODE: &str = "orderLatePaymentArrived";

static ORDER_LOCKS: LazyLock<LockMap> = LazyLock::new(LockMap::new);
/// Several provider calls (20 s each) may run under it.
const ORDER_LOCK: LockOpts = LockOpts {
    ttl_secs: 120,
    retry_secs: Some(90),
};

/// The order's payment lock (TS: the order row's `FOR UPDATE`). Not
/// re-entrant: take it once per path.
pub async fn lock_order(workspace_id: i64, order_id: i64) -> Result<LockGuard> {
    ORDER_LOCKS
        .acquire(
            &format!("order-payment:{workspace_id}:{order_id}"),
            ORDER_LOCK,
        )
        .await
        .map_err(|_| order::payment_processing())
}

// ---- orderPaymentRepository / orderPaymentDueRepository ---------------------------

fn list_by_order(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
) -> repo::Result<Vec<OrderPayment>> {
    find_many(
        conn,
        r#"WHERE "workspaceId" = ?1 AND "orderId" = ?2 ORDER BY "createdAt" DESC, "id" DESC"#,
        &[&workspace_id, &order_id],
    )
}

fn find_payment(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> repo::Result<Option<OrderPayment>> {
    find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )
}

fn find_by_reference(
    conn: &Connection,
    provider: &str,
    reference: &str,
) -> repo::Result<Option<OrderPayment>> {
    find_one(
        conn,
        r#""provider" = ?1 AND "providerReference" = ?2"#,
        &[&provider, &reference],
    )
}

/// `updateStatus`: only from the named statuses; `None` keeps a column.
#[derive(Default)]
struct PaymentUpdate<'a> {
    status: Option<&'static str>,
    transaction_id: Option<&'a str>,
    refund_id: Option<&'a str>,
    paid_at: Option<i64>,
    refunded_at: Option<i64>,
}

fn update_status(
    conn: &Connection,
    payment: &OrderPayment,
    from: &[&str],
    values: &PaymentUpdate<'_>,
) -> repo::Result<bool> {
    let from = serde_json::to_string(from).unwrap_or_default();
    let changed = conn.execute(
        r#"UPDATE "OrderPayment" SET
             "status" = COALESCE(?3, "status"),
             "transactionId" = COALESCE(?4, "transactionId"),
             "refundId" = COALESCE(?5, "refundId"),
             "paidAt" = COALESCE(?6, "paidAt"),
             "refundedAt" = COALESCE(?7, "refundedAt"),
             "updatedAt" = ?8
           WHERE "id" = ?1 AND "workspaceId" = ?2 AND "status" IN (SELECT value FROM json_each(?9))"#,
        params![
            payment.id,
            payment.workspace_id,
            values.status,
            values.transaction_id,
            values.refund_id,
            values.paid_at,
            values.refunded_at,
            now_ms(),
            from
        ],
    )?;
    Ok(changed > 0)
}

async fn write_status(
    deps: Deps<'_>,
    payment: &OrderPayment,
    from: &'static [&'static str],
    values: PaymentUpdate<'static>,
) -> Result<()> {
    let payment = payment.clone();
    tx::write(deps.db, move |tx| {
        Ok(update_status(tx, &payment, from, &values)?)
    })
    .await?;
    Ok(())
}

async fn mark_expired(deps: Deps<'_>, payment: &OrderPayment) -> Result<()> {
    write_status(
        deps,
        payment,
        &["open"],
        PaymentUpdate {
            status: Some("expired"),
            ..Default::default()
        },
    )
    .await
}

/// `postponeExpiry`: an unpaid order's deadline moves to `until`.
pub async fn postpone_expiry(
    deps: Deps<'_>,
    workspace_id: i64,
    order_id: i64,
    until: i64,
) -> Result<()> {
    tx::write(deps.db, move |tx| {
        tx.execute(
            r#"UPDATE "Order" SET "paymentExpiresAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND "paymentStatus" = 'unpaid'"#,
            params![order_id, workspace_id, until],
        )?;
        Ok(())
    })
    .await
}

/// `listOverdue`: `(workspaceId, orderId)` of online orders past their deadline.
pub async fn list_overdue(db: &chatbotx_db::Db, limit: i64) -> Result<Vec<(i64, i64)>> {
    let now = now_ms();
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare_cached(
            r#"SELECT "workspaceId", "id" FROM "Order"
               WHERE "status" = 'pending' AND "paymentStatus" = 'unpaid'
                 AND "paymentExpiresAt" IS NOT NULL AND "paymentExpiresAt" < ?1
               ORDER BY "paymentExpiresAt" LIMIT ?2"#,
        )?;
        let rows = stmt
            .query_map(params![now, limit], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

/// `listStaleAttempts`: open attempts well past their page's end.
pub async fn list_stale(db: &chatbotx_db::Db, limit: i64) -> Result<Vec<OrderPayment>> {
    let before = now_ms() - STALE_ATTEMPT_GRACE_MS;
    tx::read(db, move |conn| {
        Ok(find_many(
            conn,
            r#"WHERE "status" = 'open' AND "expiresAt" < ?1 AND "updatedAt" < ?1 ORDER BY "expiresAt" LIMIT ?2"#,
            &[&before, &limit],
        )?)
    })
    .await
}

// ---- helpers ----------------------------------------------------------------------

/// An order a payment may still settle: pending and waiting for its money.
fn is_waiting(order: Option<&Order>) -> bool {
    order.is_some_and(|o| o.status == "pending" && is_awaiting_online_payment(o))
}

fn provider_name(provider: &str) -> &'static str {
    if provider == "stripe" {
        "stripe"
    } else {
        "paypal"
    }
}

/// `callPaymentProvider`: a provider failure as the API error.
fn call<T>(provider: &str, result: PResult<T>) -> Result<T> {
    result.map_err(|error| to_exception(provider, &error))
}

fn is_not_connected(error: &BusinessError) -> bool {
    error.code() == Some("paymentProviderNotConnected")
}

fn encryption<'a>(deps: &Deps<'a>, provider: &str) -> Result<&'a Encryption> {
    deps.encryption
        .ok_or_else(|| not_connected(provider_name(provider)))
}

async fn load_gateway(
    deps: Deps<'_>,
    workspace_id: i64,
    provider: &str,
) -> Result<(Gateway, bool)> {
    let enc = encryption(&deps, provider)?.clone();
    let provider = provider.to_string();
    tx::read(deps.db, move |conn| {
        payment_provider::load_gateway(conn, &enc, workspace_id, &provider)
    })
    .await
}

/// `loadGatewayUnlessDisconnected`.
async fn gateway_unless_disconnected(deps: Deps<'_>, order: &Order) -> Result<Option<Gateway>> {
    if !is_online(&order.payment_method) {
        return Ok(None);
    }
    match load_gateway(deps, order.workspace_id, &order.payment_method).await {
        Ok((gateway, _)) => Ok(Some(gateway)),
        Err(error) if is_not_connected(&error) => {
            tracing::warn!(order_id = order.id, provider = %order.payment_method, "[payments] the order's provider is no longer connected");
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

async fn attempts(deps: Deps<'_>, workspace_id: i64, order_id: i64) -> Result<Vec<OrderPayment>> {
    tx::read(deps.db, move |conn| {
        Ok(list_by_order(conn, workspace_id, order_id)?)
    })
    .await
}

// ---- settle.ts --------------------------------------------------------------------

/// `refundAttempt`: a full refund, recorded at once. A refund the provider
/// says was already made counts as done; any other failure is
/// `orderRefundFailed`, so nothing moves while the money is still out.
pub async fn refund_attempt(
    deps: Deps<'_>,
    gateway: &Gateway,
    payment: &OrderPayment,
    transaction_id: &str,
) -> Result<()> {
    let refund_id = match gateway
        .refund(transaction_id, &format!("refund-{}", payment.id))
        .await
    {
        Ok(id) => id,
        Err(error) => {
            tracing::error!(%error, order_id = payment.order_id, order_payment_id = payment.id, provider = %payment.provider, "[payments] refund failed");
            return Err(order::refund_failed(&payment.provider));
        }
    };
    let payment = payment.clone();
    let transaction_id = transaction_id.to_string();
    tx::write(deps.db, move |tx| {
        update_status(
            tx,
            &payment,
            &["open", "paid", "expired"],
            &PaymentUpdate {
                status: Some("refunded"),
                transaction_id: Some(&transaction_id),
                refund_id: refund_id.as_deref(),
                refunded_at: Some(now_ms()),
                ..Default::default()
            },
        )?;
        Ok(())
    })
    .await
}

/// `settlePaymentsForCancel`, before the cancel's write: no money may stay
/// with the store. Every open page is closed (one paid meanwhile joins the
/// refunds or, for the expiry cancel, stops it: that customer paid in time);
/// one still settling refuses the cancel; then the refunds, each recorded at
/// once. Returns whether the order's money went back (now or earlier).
pub async fn settle_for_cancel(
    deps: Deps<'_>,
    order: &Order,
    keep_late_payment: bool,
) -> Result<bool> {
    if !is_online(&order.payment_method) {
        return Ok(false);
    }
    let all = attempts(deps, order.workspace_id, order.id).await?;
    let refunded_before = all.iter().any(|a| a.status == "refunded");
    let unsettled: Vec<_> = all
        .into_iter()
        .filter(|a| a.status == "open" || a.status == "paid")
        .collect();
    if unsettled.is_empty() {
        return Ok(refunded_before);
    }
    let Some(gateway) = gateway_unless_disconnected(deps, order).await? else {
        // Nothing can be refunded from here: a paid attempt keeps the order.
        if unsettled.iter().any(|a| a.status == "paid") {
            return Err(not_connected(provider_name(&order.payment_method)));
        }
        for attempt in &unsettled {
            mark_expired(deps, attempt).await?;
        }
        return Ok(refunded_before);
    };
    let mut refunds = Vec::new();
    for attempt in unsettled {
        if attempt.status == "paid" {
            if let Some(id) = attempt.transaction_id.clone() {
                refunds.push((attempt, id));
            }
            continue;
        }
        match call(
            &attempt.provider,
            gateway.expire_checkout(&attempt.provider_reference).await,
        )? {
            CheckoutStatus::Paid { transaction_id, .. } => {
                if keep_late_payment && order.payment_status != "paid" {
                    return Err(BusinessError::exception(
                        "A payment arrived as the order expired.",
                        LATE_PAYMENT_CODE,
                        409,
                    ));
                }
                refunds.push((attempt, transaction_id));
            }
            CheckoutStatus::Processing => return Err(order::payment_processing()),
            _ => mark_expired(deps, &attempt).await?,
        }
    }
    let any = !refunds.is_empty();
    for (payment, transaction_id) in refunds {
        refund_attempt(deps, &gateway, &payment, &transaction_id).await?;
    }
    Ok(refunded_before || any)
}

/// `assertDeletableWithPayments`: an open order paid online is cancelled
/// (refunded), never deleted; its open pages are closed first.
pub async fn assert_deletable(deps: Deps<'_>, order: &Order) -> Result<()> {
    if !(is_online(&order.payment_method) && OPEN_ORDER_STATUSES.contains(&order.status.as_str())) {
        return Ok(());
    }
    if order.payment_status == "paid" {
        return Err(order::paid_online());
    }
    let open: Vec<_> = attempts(deps, order.workspace_id, order.id)
        .await?
        .into_iter()
        .filter(|a| a.status == "open")
        .collect();
    if open.is_empty() {
        return Ok(());
    }
    let Some(gateway) = gateway_unless_disconnected(deps, order).await? else {
        return Ok(());
    };
    for attempt in open {
        let status = call(
            &attempt.provider,
            gateway.expire_checkout(&attempt.provider_reference).await,
        )?;
        if matches!(
            status,
            CheckoutStatus::Paid { .. } | CheckoutStatus::Processing
        ) {
            // The webhook records the payment; the order must be cancelled instead.
            return Err(order::paid_online());
        }
    }
    Ok(())
}

// ---- service.ts -------------------------------------------------------------------

/// `startCheckout`: the provider page the customer pays the order on. An open
/// page with time left is offered again; otherwise a new one is opened.
/// Under the order's lock: a double tap opens one page and an order being
/// cancelled gets none. Returns `(url, expiresAt ms)`.
pub async fn start_checkout(
    deps: Deps<'_>,
    workspace_id: i64,
    order_id: i64,
) -> Result<(String, i64)> {
    let _guard = lock_order(workspace_id, order_id).await?;
    let (order, attempts, workspace_name) = tx::read(deps.db, move |conn| {
        let order = order_repo::find_for_update(conn, workspace_id, order_id)?
            .ok_or_else(order::not_found)?;
        let attempts = list_by_order(conn, workspace_id, order_id)?;
        let name: Option<String> = conn
            .query_row(
                r#"SELECT "name" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .ok();
        Ok((order, attempts, name.unwrap_or_default()))
    })
    .await?;
    if !is_online(&order.payment_method) {
        return Err(order::payment_not_open("notPaidOnline"));
    }
    let provider = provider_name(&order.payment_method);
    if order.payment_status != "unpaid" {
        return Err(order::payment_not_open(&order.payment_status));
    }
    let now = now_ms();
    let deadline = match order.payment_expires_at {
        Some(at) if order.status == "pending" && at > now => at,
        _ => return Err(order::payment_not_open("expired")),
    };
    let Some(contact_id) = order.contact_id else {
        return Err(order::payment_contact_required());
    };
    if let Some(reusable) = attempts.iter().find(|a| {
        a.status == "open"
            && a.provider == provider
            && a.amount == order.total
            && a.currency == order.currency
            && a.expires_at > now + REUSE_MARGIN_MS
    }) {
        return Ok((reusable.checkout_url.clone(), deadline));
    }

    let (gateway, livemode) = load_gateway(deps, workspace_id, provider).await?;
    let payment_id = new_id();
    let (return_url, cancel_url) = return_urls(deps, &order, contact_id)?;
    // The page outlives the deadline when the provider insists on a longer
    // one; the expiry scan closes it at the deadline anyway.
    let page_expires_at = deadline.max(now + MIN_PROVIDER_PAGE_MS);
    let created = call(
        provider,
        gateway
            .create_checkout(&CreateCheckout {
                amount: order::major(order.total),
                currency: order.currency.clone(),
                description: format!("{workspace_name} #{}", order.number),
                return_url,
                cancel_url,
                expires_at: page_expires_at,
                customer_email: order.customer_email.clone(),
                metadata: PaymentMetadata {
                    workspace_id: workspace_id.to_string(),
                    order_id: order_id.to_string(),
                    order_payment_id: payment_id.to_string(),
                },
                idempotency_key: format!("order-payment-{payment_id}"),
            })
            .await,
    )?;
    let url = created.url.clone();
    let (total, currency) = (order.total, order.currency.clone());
    tx::write(deps.db, move |tx| {
        let ts = now_ms();
        tx.execute(
            r#"INSERT INTO "OrderPayment" ("id", "createdAt", "updatedAt", "orderId", "workspaceId", "provider",
                 "providerReference", "amount", "currency", "status", "livemode", "checkoutUrl", "expiresAt")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', ?9, ?10, ?11)"#,
            params![
                payment_id,
                ts,
                order_id,
                workspace_id,
                provider,
                created.reference,
                total,
                currency,
                livemode,
                created.url,
                page_expires_at
            ],
        )?;
        Ok(())
    })
    .await?;
    Ok((url, deadline))
}

/// `buildReturnUrls`: the customer's own order page. `shopToken`, not
/// `token`: PayPal appends its own `token` (its order id).
fn return_urls(deps: Deps<'_>, order: &Order, contact_id: i64) -> Result<(String, String)> {
    let enc = encryption(&deps, &order.payment_method)?;
    let token = enc
        .sign_shop_token(
            &ShopCustomerClaims {
                workspace_id: order.workspace_id.to_string(),
                contact_id: contact_id.to_string(),
                contact_inbox_id: order.contact_inbox_id.map(|v| v.to_string()),
                conversation_id: order.conversation_id.map(|v| v.to_string()),
            },
            SHOP_TOKEN_TTL_MS,
        )
        .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))?;
    let page = join_app_url(
        deps.app_url,
        &format!(
            "/shop/{}/orders/{}?shopToken={}",
            order.workspace_id,
            order.id,
            encode_uri_component(&token)
        ),
    );
    Ok((
        format!("{page}&payment=return"),
        format!("{page}&payment=cancelled"),
    ))
}

/// What reading one attempt from its provider came to.
enum SyncOutcome {
    /// `state` = the provider's, or `settled` (paid / refunded already).
    None(&'static str),
    Paid(Order),
    Refund(OrderPayment, String, Gateway),
}

/// `syncOrder`: the customer came back from the page; every open attempt is
/// read again. Best effort: the webhook settles what this could not.
pub async fn sync_order(deps: Deps<'_>, workspace_id: i64, order_id: i64) -> Result<()> {
    for attempt in attempts(deps, workspace_id, order_id)
        .await?
        .into_iter()
        .filter(|a| a.status == "open")
    {
        if let Err(error) = sync_attempt(deps, &attempt).await {
            tracing::warn!(%error, order_id, order_payment_id = attempt.id, "[payments] could not read the payment back from the provider");
        }
    }
    Ok(())
}

/// `handleWebhookEvent`: a signed event about one attempt. The provider is
/// asked again rather than trusting the event; another workspace's attempt
/// is ignored, a deleted order's only reported.
pub async fn handle_webhook_event(
    deps: Deps<'_>,
    provider: &str,
    workspace_id: i64,
    reference: &str,
    metadata: Option<&PaymentMetadata>,
) -> Result<()> {
    let (p, r) = (provider.to_string(), reference.to_string());
    let attempt = tx::read(deps.db, move |conn| Ok(find_by_reference(conn, &p, &r)?)).await?;
    let Some(attempt) = attempt else {
        if let Some(m) = metadata.filter(|m| m.workspace_id == workspace_id.to_string()) {
            tracing::warn!(order_id = %m.order_id, order_payment_id = %m.order_payment_id, provider, "[payments] a payment event for a deleted order; nothing changed");
        }
        return Ok(());
    };
    if attempt.workspace_id != workspace_id {
        tracing::warn!(
            order_payment_id = attempt.id,
            workspace_id,
            "[payments] a webhook named another workspace's payment"
        );
        return Ok(());
    }
    sync_attempt(deps, &attempt).await.map(|_| ())
}

/// `syncAttempt`: read one attempt from its provider and apply it under the
/// order's lock. An approved PayPal order is captured only while its order
/// still waits for the money, so a cancelled order is never charged.
async fn sync_attempt(deps: Deps<'_>, attempt: &OrderPayment) -> Result<SyncOutcome> {
    let (gateway, _) = load_gateway(deps, attempt.workspace_id, &attempt.provider).await?;
    let outcome = {
        let _guard = lock_order(attempt.workspace_id, attempt.order_id).await?;
        let (ws, order_id, id) = (attempt.workspace_id, attempt.order_id, attempt.id);
        let (order, payment) = tx::read(deps.db, move |conn| {
            Ok((
                order_repo::find_for_update(conn, ws, order_id)?,
                find_payment(conn, ws, id)?,
            ))
        })
        .await?;
        match payment {
            Some(p) if p.status != "paid" && p.status != "refunded" => {
                // One capture per try: a declined capture never replays its
                // failure for the next funding source.
                let capture =
                    is_waiting(order.as_ref()).then(|| format!("capture-{}-{}", p.id, new_id()));
                let status = call(
                    &p.provider,
                    gateway
                        .get_checkout_status(&p.provider_reference, capture.as_deref())
                        .await,
                )?;
                apply_status(deps, order, p, status, gateway).await?
            }
            _ => SyncOutcome::None("settled"),
        }
    };
    match &outcome {
        SyncOutcome::Paid(order) => order::after_paid(deps, order).await,
        SyncOutcome::Refund(payment, transaction_id, gateway) => {
            refund_attempt(deps, gateway, payment, transaction_id).await?;
        }
        SyncOutcome::None(_) => {}
    }
    Ok(outcome)
}

/// `applyStatus`: paid is checked to the provider's smallest unit and the
/// currency; money that does not match, or arrives for an order gone,
/// cancelled or already paid, is refunded rather than kept.
async fn apply_status(
    deps: Deps<'_>,
    order: Option<Order>,
    payment: OrderPayment,
    status: CheckoutStatus,
    gateway: Gateway,
) -> Result<SyncOutcome> {
    let (transaction_id, amount_minor, currency) = match status {
        CheckoutStatus::Paid {
            transaction_id,
            amount_minor,
            currency,
        } => (transaction_id, amount_minor, currency),
        other => {
            if other == CheckoutStatus::Closed {
                mark_expired(deps, &payment).await?;
            }
            return Ok(SyncOutcome::None(other.state()));
        }
    };
    let expected = gateway
        .to_minor_units(order::major(payment.amount), &payment.currency)
        .ok();
    if expected != Some(amount_minor) || currency != payment.currency.to_uppercase() {
        tracing::error!(order_payment_id = payment.id, order_id = payment.order_id, expected_minor = ?expected, paid_minor = amount_minor,
            expected_currency = %payment.currency, paid_currency = %currency, "[payments] the amount paid does not match the order; refunding it");
        return Ok(SyncOutcome::Refund(payment, transaction_id, gateway));
    }
    if !is_waiting(order.as_ref()) {
        tracing::warn!(
            order_payment_id = payment.id,
            order_id = payment.order_id,
            "[payments] money came in for an order no longer waiting for it; refunding it"
        );
        return Ok(SyncOutcome::Refund(payment, transaction_id, gateway));
    }
    let paid_at = now_ms();
    let (p, t) = (payment.clone(), transaction_id.clone());
    let paid = tx::write(deps.db, move |tx| {
        let Some(order) = order_repo::mark_paid(tx, p.workspace_id, p.order_id, paid_at)? else {
            return Ok(None);
        };
        update_status(
            tx,
            &p,
            &["open", "expired"],
            &PaymentUpdate {
                status: Some("paid"),
                transaction_id: Some(&t),
                paid_at: Some(paid_at),
                ..Default::default()
            },
        )?;
        Ok(Some(order))
    })
    .await?;
    Ok(match paid {
        Some(order) => SyncOutcome::Paid(order),
        None => SyncOutcome::Refund(payment, transaction_id, gateway),
    })
}

/// As `sync_attempt`; `None` when the store disconnected the provider since.
async fn sync_unless_disconnected(
    deps: Deps<'_>,
    attempt: &OrderPayment,
) -> Result<Option<SyncOutcome>> {
    match sync_attempt(deps, attempt).await {
        Ok(outcome) => Ok(Some(outcome)),
        Err(error) if is_not_connected(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

/// `settleStaleAttempt`: an open attempt well past its page's end, read once
/// more. Still settling: moved behind the others. Still open or provider
/// gone: marked expired (a PayPal order is only charged when captured).
pub async fn settle_stale_attempt(deps: Deps<'_>, attempt: &OrderPayment) -> Result<()> {
    match sync_unless_disconnected(deps, attempt).await? {
        Some(SyncOutcome::None("processing")) => postpone_stale_attempt(deps, attempt).await,
        None | Some(SyncOutcome::None("open")) => mark_expired(deps, attempt).await,
        Some(_) => Ok(()),
    }
}

/// `postponeStaleAttempt`: only touched, so the sweep looks again later.
pub async fn postpone_stale_attempt(deps: Deps<'_>, attempt: &OrderPayment) -> Result<()> {
    write_status(deps, attempt, &["open"], PaymentUpdate::default()).await
}

/// `ExpireOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpireOutcome {
    Skipped,
    Paid,
    Processing,
    Cancelled,
    RetryLater,
}

/// `expireOrder`: an online order past its deadline. A payment that came in
/// keeps it; one still settling is looked at again later; otherwise it is
/// cancelled. A provider that does not answer moves the deadline a few
/// minutes on, so one store never holds up the others.
pub async fn expire_order(
    deps: Deps<'_>,
    workspace_id: i64,
    order_id: i64,
) -> Result<ExpireOutcome> {
    let order = tx::read(deps.db, move |conn| {
        Ok(order_repo::find(conn, workspace_id, order_id)?)
    })
    .await?;
    let now = now_ms();
    let due = order
        .as_ref()
        .is_some_and(|o| is_waiting(Some(o)) && o.payment_expires_at.is_some_and(|at| at <= now));
    if !due {
        return Ok(ExpireOutcome::Skipped);
    }
    let result: Result<ExpireOutcome> = async {
        for attempt in attempts(deps, workspace_id, order_id)
            .await?
            .into_iter()
            .filter(|a| a.status == "open")
        {
            match sync_unless_disconnected(deps, &attempt).await? {
                Some(SyncOutcome::Paid(_)) => return Ok(ExpireOutcome::Paid),
                Some(SyncOutcome::None("processing")) => {
                    postpone_expiry(deps, workspace_id, order_id, now + PROCESSING_RECHECK_MS)
                        .await?;
                    return Ok(ExpireOutcome::Processing);
                }
                _ => {}
            }
        }
        order::transition(
            deps,
            workspace_id,
            order_id,
            "cancelled",
            Actor {
                kind: "system",
                user_id: None,
            },
            None,
            true,
        )
        .await?;
        Ok(ExpireOutcome::Cancelled)
    }
    .await;
    match result {
        Ok(outcome) => Ok(outcome),
        Err(error) if error.code() == Some(LATE_PAYMENT_CODE) => {
            settle_late_payment(deps, workspace_id, order_id, now).await
        }
        Err(error) if error.code() == Some("orderInvalidStatusTransition") => {
            Ok(ExpireOutcome::Skipped)
        }
        Err(error) => {
            tracing::warn!(%error, order_id, "[payments] could not expire the order; trying again later");
            postpone_expiry(deps, workspace_id, order_id, now + RETRY_LATER_MS).await?;
            Ok(ExpireOutcome::RetryLater)
        }
    }
}

/// `settleLatePayment`: an expiry cancel found a page paid as it closed.
async fn settle_late_payment(
    deps: Deps<'_>,
    workspace_id: i64,
    order_id: i64,
    now: i64,
) -> Result<ExpireOutcome> {
    let paid: Result<bool> = async {
        for attempt in attempts(deps, workspace_id, order_id)
            .await?
            .into_iter()
            .filter(|a| a.status == "open")
        {
            if let Some(SyncOutcome::Paid(_)) = sync_unless_disconnected(deps, &attempt).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    .await;
    match paid {
        Ok(true) => return Ok(ExpireOutcome::Paid),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, order_id, "[payments] could not record a payment that arrived at the deadline; trying again later");
        }
    }
    postpone_expiry(deps, workspace_id, order_id, now + RETRY_LATER_MS).await?;
    Ok(ExpireOutcome::RetryLater)
}
