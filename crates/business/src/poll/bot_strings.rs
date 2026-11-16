//! `poll/bot-strings.ts`: the poll bot texts per locale.

/// `PollBotStrings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollBotStrings {
    pub vote: &'static str,
    pub polls: &'static str,
    pub voting_started: &'static str,
    pub voting_is_over: &'static str,
    pub voting_starts_soon: &'static str,
    pub vote_registered: &'static str,
    pub no_open_polls: &'static str,
}

/// `POLL_BOT_STRINGS`, in the catalogue order.
const POLL_BOT_STRINGS: &[(&str, PollBotStrings)] = &[
    (
        "en",
        PollBotStrings {
            vote: "Vote",
            polls: "Polls",
            voting_started: "Voting started",
            voting_is_over: "Voting is over",
            voting_starts_soon: "Voting starts soon",
            vote_registered: "Your vote was successfully registered.",
            no_open_polls: "No polls are open right now.",
        },
    ),
    (
        "ar",
        PollBotStrings {
            vote: "صوّت",
            polls: "الاستطلاعات",
            voting_started: "بدأ التصويت",
            voting_is_over: "انتهى التصويت",
            voting_starts_soon: "سيبدأ التصويت قريبًا",
            vote_registered: "تم تسجيل صوتك بنجاح.",
            no_open_polls: "لا توجد استطلاعات مفتوحة حاليًا.",
        },
    ),
    (
        "az",
        PollBotStrings {
            vote: "Səs ver",
            polls: "Sorğular",
            voting_started: "Səsvermə başladı",
            voting_is_over: "Səsvermə başa çatdı",
            voting_starts_soon: "Səsvermə tezliklə başlayır",
            vote_registered: "Səsiniz uğurla qeydə alındı.",
            no_open_polls: "Hazırda açıq sorğu yoxdur.",
        },
    ),
    (
        "da",
        PollBotStrings {
            vote: "Stem",
            polls: "Afstemninger",
            voting_started: "Afstemningen er startet",
            voting_is_over: "Afstemningen er slut",
            voting_starts_soon: "Afstemningen starter snart",
            vote_registered: "Din stemme er blevet registreret.",
            no_open_polls: "Der er ingen åbne afstemninger lige nu.",
        },
    ),
    (
        "de",
        PollBotStrings {
            vote: "Abstimmen",
            polls: "Umfragen",
            voting_started: "Abstimmung gestartet",
            voting_is_over: "Abstimmung beendet",
            voting_starts_soon: "Abstimmung beginnt bald",
            vote_registered: "Deine Stimme wurde erfolgreich registriert.",
            no_open_polls: "Derzeit sind keine Umfragen offen.",
        },
    ),
    (
        "es",
        PollBotStrings {
            vote: "Votar",
            polls: "Encuestas",
            voting_started: "Votación iniciada",
            voting_is_over: "La votación ha terminado",
            voting_starts_soon: "La votación comienza pronto",
            vote_registered: "Tu voto se registró correctamente.",
            no_open_polls: "No hay encuestas abiertas en este momento.",
        },
    ),
    (
        "fi",
        PollBotStrings {
            vote: "Äänestä",
            polls: "Äänestykset",
            voting_started: "Äänestys alkoi",
            voting_is_over: "Äänestys on päättynyt",
            voting_starts_soon: "Äänestys alkaa pian",
            vote_registered: "Äänesi on rekisteröity onnistuneesti.",
            no_open_polls: "Avoimia äänestyksiä ei ole juuri nyt.",
        },
    ),
    (
        "fr",
        PollBotStrings {
            vote: "Voter",
            polls: "Sondages",
            voting_started: "Vote commencé",
            voting_is_over: "Le vote est terminé",
            voting_starts_soon: "Le vote commence bientôt",
            vote_registered: "Votre vote a bien été enregistré.",
            no_open_polls: "Aucun sondage n'est ouvert pour le moment.",
        },
    ),
    (
        "he",
        PollBotStrings {
            vote: "הצבע",
            polls: "סקרים",
            voting_started: "ההצבעה החלה",
            voting_is_over: "ההצבעה הסתיימה",
            voting_starts_soon: "ההצבעה תתחיל בקרוב",
            vote_registered: "ההצבעה שלך נרשמה בהצלחה.",
            no_open_polls: "אין סקרים פתוחים כרגע.",
        },
    ),
    (
        "id",
        PollBotStrings {
            vote: "Pilih",
            polls: "Polling",
            voting_started: "Pemungutan suara dimulai",
            voting_is_over: "Pemungutan suara telah berakhir",
            voting_starts_soon: "Pemungutan suara segera dimulai",
            vote_registered: "Suara Anda berhasil dicatat.",
            no_open_polls: "Tidak ada polling yang terbuka saat ini.",
        },
    ),
    (
        "it",
        PollBotStrings {
            vote: "Vota",
            polls: "Sondaggi",
            voting_started: "Votazione iniziata",
            voting_is_over: "La votazione è terminata",
            voting_starts_soon: "La votazione inizia presto",
            vote_registered: "Il tuo voto è stato registrato con successo.",
            no_open_polls: "Al momento non ci sono sondaggi aperti.",
        },
    ),
    (
        "ja",
        PollBotStrings {
            vote: "投票",
            polls: "投票一覧",
            voting_started: "投票開始",
            voting_is_over: "投票は終了しました",
            voting_starts_soon: "まもなく投票開始",
            vote_registered: "投票を受け付けました。",
            no_open_polls: "現在受付中の投票はありません。",
        },
    ),
    (
        "nl",
        PollBotStrings {
            vote: "Stemmen",
            polls: "Peilingen",
            voting_started: "Stemmen gestart",
            voting_is_over: "Het stemmen is voorbij",
            voting_starts_soon: "Stemmen begint binnenkort",
            vote_registered: "Je stem is succesvol geregistreerd.",
            no_open_polls: "Er zijn momenteel geen open peilingen.",
        },
    ),
    (
        "pt-BR",
        PollBotStrings {
            vote: "Votar",
            polls: "Enquetes",
            voting_started: "Votação iniciada",
            voting_is_over: "A votação terminou",
            voting_starts_soon: "A votação começa em breve",
            vote_registered: "Seu voto foi registrado com sucesso.",
            no_open_polls: "Não há enquetes abertas no momento.",
        },
    ),
    (
        "pt-PT",
        PollBotStrings {
            vote: "Votar",
            polls: "Sondagens",
            voting_started: "Votação iniciada",
            voting_is_over: "A votação terminou",
            voting_starts_soon: "A votação começa em breve",
            vote_registered: "O seu voto foi registado com sucesso.",
            no_open_polls: "Não há sondagens abertas neste momento.",
        },
    ),
    (
        "ro",
        PollBotStrings {
            vote: "Votează",
            polls: "Sondaje",
            voting_started: "Votul a început",
            voting_is_over: "Votul s-a încheiat",
            voting_starts_soon: "Votul începe în curând",
            vote_registered: "Votul tău a fost înregistrat cu succes.",
            no_open_polls: "Momentan nu există sondaje deschise.",
        },
    ),
    (
        "sv",
        PollBotStrings {
            vote: "Rösta",
            polls: "Omröstningar",
            voting_started: "Omröstningen har startat",
            voting_is_over: "Omröstningen är avslutad",
            voting_starts_soon: "Omröstningen startar snart",
            vote_registered: "Din röst har registrerats.",
            no_open_polls: "Det finns inga öppna omröstningar just nu.",
        },
    ),
    (
        "tr",
        PollBotStrings {
            vote: "Oy ver",
            polls: "Anketler",
            voting_started: "Oylama başladı",
            voting_is_over: "Oylama sona erdi",
            voting_starts_soon: "Oylama yakında başlıyor",
            vote_registered: "Oyunuz başarıyla kaydedildi.",
            no_open_polls: "Şu anda açık anket yok.",
        },
    ),
    (
        "vi",
        PollBotStrings {
            vote: "Bình chọn",
            polls: "Cuộc bình chọn",
            voting_started: "Đã bắt đầu bình chọn",
            voting_is_over: "Bình chọn đã kết thúc",
            voting_starts_soon: "Sắp bắt đầu bình chọn",
            vote_registered: "Phiếu bình chọn của bạn đã được ghi nhận.",
            no_open_polls: "Hiện không có cuộc bình chọn nào đang mở.",
        },
    ),
    (
        "zh-CN",
        PollBotStrings {
            vote: "投票",
            polls: "投票列表",
            voting_started: "投票已开始",
            voting_is_over: "投票已结束",
            voting_starts_soon: "投票即将开始",
            vote_registered: "您的投票已成功记录。",
            no_open_polls: "目前没有进行中的投票。",
        },
    ),
    (
        "zh-TW",
        PollBotStrings {
            vote: "投票",
            polls: "投票列表",
            voting_started: "投票已開始",
            voting_is_over: "投票已結束",
            voting_starts_soon: "投票即將開始",
            vote_registered: "您的投票已成功記錄。",
            no_open_polls: "目前沒有進行中的投票。",
        },
    ),
];

fn base_language(locale: &str) -> String {
    locale.split(['-', '_']).next().unwrap_or("").to_lowercase()
}

/// `getPollBotStrings`: the exact locale, then its base language, then `en`.
pub fn get_poll_bot_strings(language: &str) -> &'static PollBotStrings {
    if let Some((_, strings)) = POLL_BOT_STRINGS.iter().find(|(key, _)| *key == language) {
        return strings;
    }
    let base = base_language(language);
    POLL_BOT_STRINGS
        .iter()
        .find(|(key, _)| base_language(key) == base)
        .or_else(|| POLL_BOT_STRINGS.iter().find(|(key, _)| *key == "en"))
        .map(|(_, strings)| strings)
        .unwrap_or(&POLL_BOT_STRINGS[0].1)
}
