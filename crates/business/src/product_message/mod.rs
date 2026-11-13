//! `product-message/*`: the products a flow step may send and how a step's
//! sections resolve against them. Only the parts the public API and their
//! tests need are ported here (`listSendable`, `resolveProductMessage`).

pub mod resolve;
pub mod service;
