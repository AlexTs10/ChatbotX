//! The `Intl` / `date-fns` / `date-fns-tz` behavior the date helpers rely
//! on: time zone names as `Intl` accepts them, offsets read from the IANA
//! database (`chrono-tz`), `fromZonedTime`'s offset fix, `toDate` string
//! parsing and `format` for the patterns the code uses. The process zone
//! is UTC.

use std::str::FromStr;
use std::sync::OnceLock;

use chrono::{Offset, TimeZone};
use regex::Regex;

use crate::js::{self, UtcFields};

/// `Intl.supportedValuesOf("timeZone")` of Node 24.22.1 (ICU 78.3).
pub const INTL_SUPPORTED_TIME_ZONES: &[&str] = &[
    "Africa/Abidjan",
    "Africa/Accra",
    "Africa/Addis_Ababa",
    "Africa/Algiers",
    "Africa/Asmera",
    "Africa/Bamako",
    "Africa/Bangui",
    "Africa/Banjul",
    "Africa/Bissau",
    "Africa/Blantyre",
    "Africa/Brazzaville",
    "Africa/Bujumbura",
    "Africa/Cairo",
    "Africa/Casablanca",
    "Africa/Ceuta",
    "Africa/Conakry",
    "Africa/Dakar",
    "Africa/Dar_es_Salaam",
    "Africa/Djibouti",
    "Africa/Douala",
    "Africa/El_Aaiun",
    "Africa/Freetown",
    "Africa/Gaborone",
    "Africa/Harare",
    "Africa/Johannesburg",
    "Africa/Juba",
    "Africa/Kampala",
    "Africa/Khartoum",
    "Africa/Kigali",
    "Africa/Kinshasa",
    "Africa/Lagos",
    "Africa/Libreville",
    "Africa/Lome",
    "Africa/Luanda",
    "Africa/Lubumbashi",
    "Africa/Lusaka",
    "Africa/Malabo",
    "Africa/Maputo",
    "Africa/Maseru",
    "Africa/Mbabane",
    "Africa/Mogadishu",
    "Africa/Monrovia",
    "Africa/Nairobi",
    "Africa/Ndjamena",
    "Africa/Niamey",
    "Africa/Nouakchott",
    "Africa/Ouagadougou",
    "Africa/Porto-Novo",
    "Africa/Sao_Tome",
    "Africa/Tripoli",
    "Africa/Tunis",
    "Africa/Windhoek",
    "America/Adak",
    "America/Anchorage",
    "America/Anguilla",
    "America/Antigua",
    "America/Araguaina",
    "America/Argentina/La_Rioja",
    "America/Argentina/Rio_Gallegos",
    "America/Argentina/Salta",
    "America/Argentina/San_Juan",
    "America/Argentina/San_Luis",
    "America/Argentina/Tucuman",
    "America/Argentina/Ushuaia",
    "America/Aruba",
    "America/Asuncion",
    "America/Bahia",
    "America/Bahia_Banderas",
    "America/Barbados",
    "America/Belem",
    "America/Belize",
    "America/Blanc-Sablon",
    "America/Boa_Vista",
    "America/Bogota",
    "America/Boise",
    "America/Buenos_Aires",
    "America/Cambridge_Bay",
    "America/Campo_Grande",
    "America/Cancun",
    "America/Caracas",
    "America/Catamarca",
    "America/Cayenne",
    "America/Cayman",
    "America/Chicago",
    "America/Chihuahua",
    "America/Ciudad_Juarez",
    "America/Coral_Harbour",
    "America/Cordoba",
    "America/Costa_Rica",
    "America/Coyhaique",
    "America/Creston",
    "America/Cuiaba",
    "America/Curacao",
    "America/Danmarkshavn",
    "America/Dawson",
    "America/Dawson_Creek",
    "America/Denver",
    "America/Detroit",
    "America/Dominica",
    "America/Edmonton",
    "America/Eirunepe",
    "America/El_Salvador",
    "America/Fort_Nelson",
    "America/Fortaleza",
    "America/Glace_Bay",
    "America/Godthab",
    "America/Goose_Bay",
    "America/Grand_Turk",
    "America/Grenada",
    "America/Guadeloupe",
    "America/Guatemala",
    "America/Guayaquil",
    "America/Guyana",
    "America/Halifax",
    "America/Havana",
    "America/Hermosillo",
    "America/Indiana/Knox",
    "America/Indiana/Marengo",
    "America/Indiana/Petersburg",
    "America/Indiana/Tell_City",
    "America/Indiana/Vevay",
    "America/Indiana/Vincennes",
    "America/Indiana/Winamac",
    "America/Indianapolis",
    "America/Inuvik",
    "America/Iqaluit",
    "America/Jamaica",
    "America/Jujuy",
    "America/Juneau",
    "America/Kentucky/Monticello",
    "America/Kralendijk",
    "America/La_Paz",
    "America/Lima",
    "America/Los_Angeles",
    "America/Louisville",
    "America/Lower_Princes",
    "America/Maceio",
    "America/Managua",
    "America/Manaus",
    "America/Marigot",
    "America/Martinique",
    "America/Matamoros",
    "America/Mazatlan",
    "America/Mendoza",
    "America/Menominee",
    "America/Merida",
    "America/Metlakatla",
    "America/Mexico_City",
    "America/Miquelon",
    "America/Moncton",
    "America/Monterrey",
    "America/Montevideo",
    "America/Montserrat",
    "America/Nassau",
    "America/New_York",
    "America/Nome",
    "America/Noronha",
    "America/North_Dakota/Beulah",
    "America/North_Dakota/Center",
    "America/North_Dakota/New_Salem",
    "America/Ojinaga",
    "America/Panama",
    "America/Paramaribo",
    "America/Phoenix",
    "America/Port-au-Prince",
    "America/Port_of_Spain",
    "America/Porto_Velho",
    "America/Puerto_Rico",
    "America/Punta_Arenas",
    "America/Rankin_Inlet",
    "America/Recife",
    "America/Regina",
    "America/Resolute",
    "America/Rio_Branco",
    "America/Santarem",
    "America/Santiago",
    "America/Santo_Domingo",
    "America/Sao_Paulo",
    "America/Scoresbysund",
    "America/Sitka",
    "America/St_Barthelemy",
    "America/St_Johns",
    "America/St_Kitts",
    "America/St_Lucia",
    "America/St_Thomas",
    "America/St_Vincent",
    "America/Swift_Current",
    "America/Tegucigalpa",
    "America/Thule",
    "America/Tijuana",
    "America/Toronto",
    "America/Tortola",
    "America/Vancouver",
    "America/Whitehorse",
    "America/Winnipeg",
    "America/Yakutat",
    "Antarctica/Casey",
    "Antarctica/Davis",
    "Antarctica/DumontDUrville",
    "Antarctica/Macquarie",
    "Antarctica/Mawson",
    "Antarctica/McMurdo",
    "Antarctica/Palmer",
    "Antarctica/Rothera",
    "Antarctica/Syowa",
    "Antarctica/Troll",
    "Antarctica/Vostok",
    "Arctic/Longyearbyen",
    "Asia/Aden",
    "Asia/Almaty",
    "Asia/Amman",
    "Asia/Anadyr",
    "Asia/Aqtau",
    "Asia/Aqtobe",
    "Asia/Ashgabat",
    "Asia/Atyrau",
    "Asia/Baghdad",
    "Asia/Bahrain",
    "Asia/Baku",
    "Asia/Bangkok",
    "Asia/Barnaul",
    "Asia/Beirut",
    "Asia/Bishkek",
    "Asia/Brunei",
    "Asia/Calcutta",
    "Asia/Chita",
    "Asia/Colombo",
    "Asia/Damascus",
    "Asia/Dhaka",
    "Asia/Dili",
    "Asia/Dubai",
    "Asia/Dushanbe",
    "Asia/Famagusta",
    "Asia/Gaza",
    "Asia/Hebron",
    "Asia/Hong_Kong",
    "Asia/Hovd",
    "Asia/Irkutsk",
    "Asia/Jakarta",
    "Asia/Jayapura",
    "Asia/Jerusalem",
    "Asia/Kabul",
    "Asia/Kamchatka",
    "Asia/Karachi",
    "Asia/Katmandu",
    "Asia/Khandyga",
    "Asia/Krasnoyarsk",
    "Asia/Kuala_Lumpur",
    "Asia/Kuching",
    "Asia/Kuwait",
    "Asia/Macau",
    "Asia/Magadan",
    "Asia/Makassar",
    "Asia/Manila",
    "Asia/Muscat",
    "Asia/Nicosia",
    "Asia/Novokuznetsk",
    "Asia/Novosibirsk",
    "Asia/Omsk",
    "Asia/Oral",
    "Asia/Phnom_Penh",
    "Asia/Pontianak",
    "Asia/Pyongyang",
    "Asia/Qatar",
    "Asia/Qostanay",
    "Asia/Qyzylorda",
    "Asia/Rangoon",
    "Asia/Riyadh",
    "Asia/Saigon",
    "Asia/Sakhalin",
    "Asia/Samarkand",
    "Asia/Seoul",
    "Asia/Shanghai",
    "Asia/Singapore",
    "Asia/Srednekolymsk",
    "Asia/Taipei",
    "Asia/Tashkent",
    "Asia/Tbilisi",
    "Asia/Tehran",
    "Asia/Thimphu",
    "Asia/Tokyo",
    "Asia/Tomsk",
    "Asia/Ulaanbaatar",
    "Asia/Urumqi",
    "Asia/Ust-Nera",
    "Asia/Vientiane",
    "Asia/Vladivostok",
    "Asia/Yakutsk",
    "Asia/Yekaterinburg",
    "Asia/Yerevan",
    "Atlantic/Azores",
    "Atlantic/Bermuda",
    "Atlantic/Canary",
    "Atlantic/Cape_Verde",
    "Atlantic/Faeroe",
    "Atlantic/Madeira",
    "Atlantic/Reykjavik",
    "Atlantic/South_Georgia",
    "Atlantic/St_Helena",
    "Atlantic/Stanley",
    "Australia/Adelaide",
    "Australia/Brisbane",
    "Australia/Broken_Hill",
    "Australia/Darwin",
    "Australia/Eucla",
    "Australia/Hobart",
    "Australia/Lindeman",
    "Australia/Lord_Howe",
    "Australia/Melbourne",
    "Australia/Perth",
    "Australia/Sydney",
    "Europe/Amsterdam",
    "Europe/Andorra",
    "Europe/Astrakhan",
    "Europe/Athens",
    "Europe/Belgrade",
    "Europe/Berlin",
    "Europe/Bratislava",
    "Europe/Brussels",
    "Europe/Bucharest",
    "Europe/Budapest",
    "Europe/Busingen",
    "Europe/Chisinau",
    "Europe/Copenhagen",
    "Europe/Dublin",
    "Europe/Gibraltar",
    "Europe/Guernsey",
    "Europe/Helsinki",
    "Europe/Isle_of_Man",
    "Europe/Istanbul",
    "Europe/Jersey",
    "Europe/Kaliningrad",
    "Europe/Kiev",
    "Europe/Kirov",
    "Europe/Lisbon",
    "Europe/Ljubljana",
    "Europe/London",
    "Europe/Luxembourg",
    "Europe/Madrid",
    "Europe/Malta",
    "Europe/Mariehamn",
    "Europe/Minsk",
    "Europe/Monaco",
    "Europe/Moscow",
    "Europe/Oslo",
    "Europe/Paris",
    "Europe/Podgorica",
    "Europe/Prague",
    "Europe/Riga",
    "Europe/Rome",
    "Europe/Samara",
    "Europe/San_Marino",
    "Europe/Sarajevo",
    "Europe/Saratov",
    "Europe/Simferopol",
    "Europe/Skopje",
    "Europe/Sofia",
    "Europe/Stockholm",
    "Europe/Tallinn",
    "Europe/Tirane",
    "Europe/Ulyanovsk",
    "Europe/Vaduz",
    "Europe/Vatican",
    "Europe/Vienna",
    "Europe/Vilnius",
    "Europe/Volgograd",
    "Europe/Warsaw",
    "Europe/Zagreb",
    "Europe/Zurich",
    "Indian/Antananarivo",
    "Indian/Chagos",
    "Indian/Christmas",
    "Indian/Cocos",
    "Indian/Comoro",
    "Indian/Kerguelen",
    "Indian/Mahe",
    "Indian/Maldives",
    "Indian/Mauritius",
    "Indian/Mayotte",
    "Indian/Reunion",
    "Pacific/Apia",
    "Pacific/Auckland",
    "Pacific/Bougainville",
    "Pacific/Chatham",
    "Pacific/Easter",
    "Pacific/Efate",
    "Pacific/Enderbury",
    "Pacific/Fakaofo",
    "Pacific/Fiji",
    "Pacific/Funafuti",
    "Pacific/Galapagos",
    "Pacific/Gambier",
    "Pacific/Guadalcanal",
    "Pacific/Guam",
    "Pacific/Honolulu",
    "Pacific/Kiritimati",
    "Pacific/Kosrae",
    "Pacific/Kwajalein",
    "Pacific/Majuro",
    "Pacific/Marquesas",
    "Pacific/Midway",
    "Pacific/Nauru",
    "Pacific/Niue",
    "Pacific/Norfolk",
    "Pacific/Noumea",
    "Pacific/Pago_Pago",
    "Pacific/Palau",
    "Pacific/Pitcairn",
    "Pacific/Ponape",
    "Pacific/Port_Moresby",
    "Pacific/Rarotonga",
    "Pacific/Saipan",
    "Pacific/Tahiti",
    "Pacific/Tarawa",
    "Pacific/Tongatapu",
    "Pacific/Truk",
    "Pacific/Wake",
    "Pacific/Wallis",
];

/// A time zone `Intl` accepts: an IANA name (any case) or a fixed offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Named(chrono_tz::Tz),
    /// East-positive offset in ms (`+07:00` = 25,200,000).
    Fixed(i64),
}

fn offset_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^([+-])([0-9]{2})(?::?([0-9]{2}))?$").expect("valid regex"))
}

impl Zone {
    /// `new Intl.DateTimeFormat(…, { timeZone: name })` does not throw.
    pub fn parse(name: &str) -> Option<Zone> {
        if let Some(caps) = offset_name_re().captures(name) {
            let hours: i64 = caps[2].parse().ok()?;
            let minutes: i64 = caps.get(3).map_or(Some(0), |m| m.as_str().parse().ok())?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let sign = if &caps[1] == "-" { -1 } else { 1 };
            return Some(Zone::Fixed(sign * (hours * 3_600_000 + minutes * 60_000)));
        }
        if name.is_empty() || name.trim() != name {
            return None;
        }
        chrono_tz::Tz::from_str(name)
            .ok()
            .or_else(|| zone_by_lower_name(&name.to_ascii_lowercase()))
            .map(Zone::Named)
    }

    /// The UTC zone.
    pub fn utc() -> Zone {
        Zone::Named(chrono_tz::UTC)
    }

    /// `local - utc` in ms at the instant `utc_ms`.
    pub fn offset_ms(&self, utc_ms: i64) -> i64 {
        match self {
            Zone::Fixed(offset) => *offset,
            Zone::Named(tz) => {
                let Some(instant) = chrono::DateTime::from_timestamp_millis(utc_ms) else {
                    return 0;
                };
                i64::from(
                    tz.offset_from_utc_datetime(&instant.naive_utc())
                        .fix()
                        .local_minus_utc(),
                ) * 1000
            }
        }
    }

    /// The wall-clock fields at `utc_ms` (`toZonedTime`).
    pub fn wall_clock(&self, utc_ms: i64) -> UtcFields {
        js::utc_fields(utc_ms + self.offset_ms(utc_ms))
    }

    /// `fromZonedTime` / `toDate(…, { timeZone })` for a wall clock given
    /// as ms-as-if-UTC: date-fns-tz's `fixOffset` (an ambiguous time takes
    /// the offset that holds at the guess; a gap keeps the larger offset).
    pub fn from_wall_clock(&self, local_ms: i64) -> i64 {
        let offset = self.offset_ms(local_ms);
        let mut utc_guess = local_ms - offset;
        let o2 = self.offset_ms(utc_guess);
        if offset == o2 {
            return local_ms - offset;
        }
        utc_guess -= o2 - offset;
        let o3 = self.offset_ms(utc_guess);
        if o2 == o3 {
            return local_ms - o2;
        }
        local_ms - o2.max(o3)
    }
}

fn zone_by_lower_name(lower: &str) -> Option<chrono_tz::Tz> {
    static BY_LOWER: OnceLock<std::collections::HashMap<String, chrono_tz::Tz>> = OnceLock::new();
    BY_LOWER
        .get_or_init(|| {
            chrono_tz::TZ_VARIANTS
                .iter()
                .map(|tz| (tz.name().to_ascii_lowercase(), *tz))
                .collect()
        })
        .get(lower)
        .copied()
}

/// Whether `Intl` accepts `name` as a time zone.
pub fn is_intl_time_zone(name: &str) -> bool {
    Zone::parse(name).is_some()
}

fn tz_suffix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(Z|[+-][0-9]{2}(?::?[0-9]{2})?| UTC| [a-zA-Z]+/[a-zA-Z_]+(?:/[a-zA-Z_]+)?)$")
            .expect("valid regex")
    })
}

/// Whether a string carries a zone of its own for date-fns-tz (`tzPattern`).
pub fn has_tz_suffix(text: &str) -> bool {
    tz_suffix_re().is_match(text)
}

/// `tzParseTimezone` for a zone string: the offset (ms, east positive) of
/// the wall clock `local_ms`, or `None` (NaN).
fn zone_offset_for_wall_clock(zone: &str, local_ms: i64) -> Option<i64> {
    if zone.is_empty() || zone == "Z" {
        return Some(0);
    }
    let zone = Zone::parse(zone)?;
    Some(local_ms - zone.from_wall_clock(local_ms))
}

fn parse_fraction_ms(whole: &str, fraction: Option<&str>, unit_ms: f64) -> f64 {
    let text = match fraction {
        Some(f) => format!("{whole}.{}", &f[1..]),
        None => whole.to_owned(),
    };
    text.parse::<f64>().unwrap_or(f64::NAN) * unit_ms
}

/// date-fns-tz `toDate(text, { timeZone })`: ISO date (`yyyy`, `yyyy-MM`,
/// `yyyy-MM-dd`, `yyyyMMdd`), `T` or space, `HH[:mm[:ss[.f]]]`, then an
/// optional zone (`Z`, `±hh[:mm]`, ` UTC`, ` Area/City`). A naive value is
/// read in `time_zone` (UTC when `None`). `None` is an Invalid Date.
pub fn tz_to_date(text: &str, time_zone: Option<&str>) -> Option<i64> {
    static DATE_TIME: OnceLock<Regex> = OnceLock::new();
    static DATE: OnceLock<Regex> = OnceLock::new();
    let date_time =
        DATE_TIME.get_or_init(|| Regex::new("^([0-9W+-]+)(T| )(.*)").expect("valid regex"));
    let date_only = DATE.get_or_init(|| Regex::new("^([0-9W+-]+)(.*)").expect("valid regex"));
    let (date_part, time_string) = if let Some(c) = date_time.captures(text) {
        (
            c.get(1).map(|m| m.as_str()),
            c.get(3).map_or("", |m| m.as_str()).to_owned(),
        )
    } else if let Some(c) = date_only.captures(text) {
        (
            c.get(1).map(|m| m.as_str()),
            c.get(2).map_or("", |m| m.as_str()).to_owned(),
        )
    } else {
        (None, text.to_owned())
    };
    let (time, zone) = if time_string.is_empty() {
        (None, None)
    } else if let Some(m) = tz_suffix_re().captures(&time_string).and_then(|c| c.get(1)) {
        let token = m.as_str();
        (
            Some(time_string.replacen(token, "", 1)),
            Some(token.trim().to_owned()),
        )
    } else {
        (Some(time_string.clone()), None)
    };
    let date_part = date_part?;
    // parseYear (additionalDigits = 2).
    static YYYY: OnceLock<Regex> = OnceLock::new();
    static YYYYYY: OnceLock<Regex> = OnceLock::new();
    let (year, rest) = if let Some(m) = YYYY
        .get_or_init(|| Regex::new("^([0-9]{4})").expect("re"))
        .captures(date_part)
    {
        (m[1].parse::<i64>().ok()?, &date_part[4..])
    } else if let Some(m) = YYYYYY
        .get_or_init(|| Regex::new("^([+-][0-9]{6})").expect("re"))
        .captures(date_part)
    {
        (m[1].parse::<i64>().ok()?, &date_part[7..])
    } else {
        return None;
    };
    static MM: OnceLock<Regex> = OnceLock::new();
    static MMDD: OnceLock<Regex> = OnceLock::new();
    let date_ms = if rest.is_empty() {
        js::date_utc(year, 0, 1, 0, 0, 0, 0)
    } else if let Some(c) = MM
        .get_or_init(|| Regex::new("^-([0-9]{2})$").expect("re"))
        .captures(rest)
    {
        let month: i64 = c[1].parse().ok()?;
        if !(1..=12).contains(&month) {
            return None;
        }
        js::date_utc(year, month - 1, 1, 0, 0, 0, 0)
    } else if let Some(c) = MMDD
        .get_or_init(|| Regex::new("^-?([0-9]{2})-?([0-9]{2})$").expect("re"))
        .captures(rest)
    {
        let month: i64 = c[1].parse().ok()?;
        let day: i64 = c[2].parse().ok()?;
        if !(1..=12).contains(&month)
            || day < 1
            || day > i64::from(js::days_in_month(year, month as u32))
        {
            return None;
        }
        js::date_utc(year, month - 1, day, 0, 0, 0, 0)
    } else {
        return None;
    };
    let mut time_ms = 0f64;
    if let Some(time) = time.as_deref().filter(|t| !t.is_empty()) {
        static HH: OnceLock<Regex> = OnceLock::new();
        static HHMM: OnceLock<Regex> = OnceLock::new();
        static HHMMSS: OnceLock<Regex> = OnceLock::new();
        let hh = HH.get_or_init(|| Regex::new(r"^([0-9]{2})([.,][0-9]*)?$").expect("re"));
        let hhmm =
            HHMM.get_or_init(|| Regex::new(r"^([0-9]{2}):?([0-9]{2})([.,][0-9]*)?$").expect("re"));
        let hhmmss = HHMMSS.get_or_init(|| {
            Regex::new(r"^([0-9]{2}):?([0-9]{2}):?([0-9]{2})([.,][0-9]*)?$").expect("re")
        });
        let frac = |m: Option<regex::Match<'_>>| m.map(|m| m.as_str().replace(',', "."));
        if let Some(c) = hh.captures(time) {
            let hours = parse_fraction_ms(&c[1], frac(c.get(2)).as_deref(), 1.0);
            if !(0.0..25.0).contains(&hours) {
                return None;
            }
            time_ms = (hours % 24.0) * 3_600_000.0;
        } else if let Some(c) = hhmm.captures(time) {
            let hours: f64 = c[1].parse().ok()?;
            let minutes = parse_fraction_ms(&c[2], frac(c.get(3)).as_deref(), 1.0);
            if !(0.0..25.0).contains(&hours) || !(0.0..60.0).contains(&minutes) {
                return None;
            }
            time_ms = (hours % 24.0) * 3_600_000.0 + minutes * 60_000.0;
        } else if let Some(c) = hhmmss.captures(time) {
            let hours: f64 = c[1].parse().ok()?;
            let minutes: f64 = c[2].parse().ok()?;
            let seconds = parse_fraction_ms(&c[3], frac(c.get(4)).as_deref(), 1.0);
            if !(0.0..25.0).contains(&hours)
                || !(0.0..60.0).contains(&minutes)
                || !(0.0..60.0).contains(&seconds)
            {
                return None;
            }
            time_ms = (hours % 24.0) * 3_600_000.0 + minutes * 60_000.0 + seconds * 1000.0;
        } else {
            return None;
        }
    }
    let local = date_ms as f64 + time_ms;
    if !local.is_finite() {
        return None;
    }
    let local_ms = local.trunc() as i64;
    let offset = match zone.as_deref().or(time_zone) {
        Some(z) => zone_offset_for_wall_clock(z, local_ms)?,
        None => 0,
    };
    let result = local_ms - offset;
    (result.abs() <= js::MAX_TIME_MS).then_some(result)
}

/// A date-fns `Date` argument: an instant, or a string (`toDate` parses it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateArg<'a> {
    Ms(i64),
    Text(&'a str),
}

/// date-fns `format` of the wall-clock fields `f` (`offset_ms` is the zone
/// offset `XXX` prints). Supports the tokens the ported code uses:
/// `yyyy MM M dd d HH H hh h mm m ss s SSS a XXX xxx` and `'quoted'` text.
pub fn format_fields(f: &UtcFields, offset_ms: i64, pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            let mut j = i + 1;
            let mut text = String::new();
            while j < chars.len() {
                if chars[j] == '\'' {
                    if chars.get(j + 1) == Some(&'\'') {
                        text.push('\'');
                        j += 2;
                        continue;
                    }
                    break;
                }
                text.push(chars[j]);
                j += 1;
            }
            if j == i + 1 && chars.get(j) == Some(&'\'') {
                out.push('\'');
            } else {
                out.push_str(&text);
            }
            i = j + 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let hour12 = if f.hour % 12 == 0 { 12 } else { f.hour % 12 };
        let token = match (c, n) {
            ('y', 2) => format!("{:02}", f.year.rem_euclid(100)),
            ('y', n) => {
                let y = if f.year <= 0 { 1 - f.year } else { f.year };
                format!("{:0width$}", y, width = n)
            }
            ('M', 1) => f.month.to_string(),
            ('M', _) => format!("{:02}", f.month),
            ('d', 1) => f.day.to_string(),
            ('d', _) => format!("{:02}", f.day),
            ('H', 1) => f.hour.to_string(),
            ('H', _) => format!("{:02}", f.hour),
            ('h', 1) => hour12.to_string(),
            ('h', _) => format!("{hour12:02}"),
            ('m', 1) => f.minute.to_string(),
            ('m', _) => format!("{:02}", f.minute),
            ('s', 1) => f.second.to_string(),
            ('s', _) => format!("{:02}", f.second),
            ('S', n) => {
                let ms = format!("{:03}", f.millisecond);
                if n <= 3 {
                    ms[..n].to_owned()
                } else {
                    format!("{ms:0<n$}")
                }
            }
            ('a', _) => if f.hour < 12 { "AM" } else { "PM" }.to_owned(),
            ('X', _) if offset_ms == 0 => "Z".to_owned(),
            ('X', n) | ('x', n) => format_offset(offset_ms, n),
            _ => std::iter::repeat_n(c, n).collect(),
        };
        out.push_str(&token);
        i += n;
    }
    out
}

fn format_offset(offset_ms: i64, width: usize) -> String {
    let sign = if offset_ms < 0 { '-' } else { '+' };
    let total_minutes = offset_ms.abs() / 60_000;
    let (h, m) = (total_minutes / 60, total_minutes % 60);
    match width {
        1 if m == 0 => format!("{sign}{h:02}"),
        1 | 2 => format!("{sign}{h:02}{m:02}"),
        _ => format!("{sign}{h:02}:{m:02}"),
    }
}

/// The error of `formatInTimeZone` / `fromZonedTime` (`RangeError`).
pub const INVALID_TIME_VALUE: &str = "Invalid time value";

/// date-fns-tz `formatInTimeZone(date, timeZone, pattern)`.
pub fn format_in_time_zone(
    date: DateArg<'_>,
    time_zone: &str,
    pattern: &str,
) -> Result<String, &'static str> {
    let zone = Zone::parse(time_zone).ok_or(INVALID_TIME_VALUE)?;
    let instant = match date {
        DateArg::Ms(ms) => ms,
        DateArg::Text(text) => tz_to_date(text, Some(time_zone)).ok_or(INVALID_TIME_VALUE)?,
    };
    if instant.abs() > js::MAX_TIME_MS {
        return Err(INVALID_TIME_VALUE);
    }
    let offset = zone.offset_ms(instant);
    Ok(format_fields(
        &js::utc_fields(instant + offset),
        offset,
        pattern,
    ))
}

/// date-fns `format(date, pattern)` in the process zone (UTC).
pub fn format_utc(ms: i64, pattern: &str) -> String {
    format_fields(&js::utc_fields(ms), 0, pattern)
}

/// date-fns-tz `fromZonedTime(text, timeZone)`.
pub fn from_zoned_time(text: &str, time_zone: &str) -> Result<i64, &'static str> {
    if has_tz_suffix(text) {
        // A value with its own zone: its instant re-read as a wall clock in
        // the target zone (date-fns-tz's behavior for this input).
        let instant = tz_to_date(text, None).ok_or(INVALID_TIME_VALUE)?;
        let zone = Zone::parse(time_zone).ok_or(INVALID_TIME_VALUE)?;
        return Ok(zone.from_wall_clock(instant));
    }
    tz_to_date(text, Some(time_zone)).ok_or(INVALID_TIME_VALUE)
}

/// date-fns-tz `fromZonedTime(date, timeZone)` for wall-clock fields
/// (a `Date` whose local getters are the wall clock).
pub fn from_zoned_ms(local_ms: i64, time_zone: &str) -> Result<i64, &'static str> {
    let zone = Zone::parse(time_zone).ok_or(INVALID_TIME_VALUE)?;
    Ok(zone.from_wall_clock(local_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_zoned_time_matches_date_fns_tz_probes() {
        // Node 24 + date-fns-tz 3.2 (P3.1 probe): the gap maps backwards,
        // the overlap takes the earlier instant.
        assert_eq!(
            js::to_iso_string(from_zoned_time("2026-03-08T02:30:00", "America/New_York").unwrap())
                .unwrap(),
            "2026-03-08T06:30:00.000Z"
        );
        assert_eq!(
            js::to_iso_string(from_zoned_time("2026-11-01T01:30:00", "America/New_York").unwrap())
                .unwrap(),
            "2026-11-01T05:30:00.000Z"
        );
        assert!(from_zoned_time("2026-02-30T00:00:00", "Asia/Ho_Chi_Minh").is_err());
        assert!(from_zoned_time("2026-07-22T15:30", "not-a-zone").is_err());
        assert_eq!(
            js::to_iso_string(
                from_zoned_time("2026-07-22T15:30:00+02:00", "Asia/Ho_Chi_Minh").unwrap()
            )
            .unwrap(),
            "2026-07-22T06:30:00.000Z"
        );
    }

    #[test]
    fn format_in_time_zone_matches_probes() {
        let vn = "Asia/Ho_Chi_Minh";
        let f = "yyyy-MM-dd HH:mm:ss";
        assert_eq!(
            format_in_time_zone(DateArg::Text("not-a-timestamp"), vn, f),
            Err(INVALID_TIME_VALUE)
        );
        assert_eq!(
            format_in_time_zone(DateArg::Text("2026-07-22 15:30"), vn, f).unwrap(),
            "2026-07-22 15:30:00"
        );
        assert_eq!(
            format_in_time_zone(DateArg::Text("2026-07-22T08:30:00.000Z"), vn, f).unwrap(),
            "2026-07-22 15:30:00"
        );
        assert_eq!(
            format_in_time_zone(DateArg::Text("2026-07-22"), vn, f).unwrap(),
            "2026-07-22 00:00:00"
        );
        let at = js::parse_date("2026-07-22T08:30:00Z").unwrap();
        assert_eq!(
            format_in_time_zone(DateArg::Ms(at), "not-a-zone", f),
            Err(INVALID_TIME_VALUE)
        );
        assert_eq!(
            format_in_time_zone(DateArg::Ms(at), "+07:00", "yyyy-MM-dd'T'HH:mm:ssXXX").unwrap(),
            "2026-07-22T15:30:00+07:00"
        );
        assert_eq!(
            format_in_time_zone(DateArg::Ms(at), "UTC", "yyyy-MM-dd'T'HH:mm:ssXXX").unwrap(),
            "2026-07-22T08:30:00Z"
        );
        assert_eq!(
            format_in_time_zone(DateArg::Text("2026-07-22T15:30:00+07:00"), "UTC", f).unwrap(),
            "2026-07-22 08:30:00"
        );
        assert_eq!(
            format_in_time_zone(DateArg::Ms(at), "asia/ho_chi_minh", f).unwrap(),
            "2026-07-22 15:30:00"
        );
    }

    #[test]
    fn intl_accepts_offsets_and_any_case() {
        for ok in [
            "+07:00",
            "asia/ho_chi_minh",
            "UTC",
            "utc",
            "GMT",
            "Etc/GMT+7",
            "Asia/Saigon",
            "EST",
            "US/Eastern",
            "-0700",
            "+07",
        ] {
            assert!(is_intl_time_zone(ok), "{ok}");
        }
        for bad in ["US", "Mars/Olympus", ""] {
            assert!(!is_intl_time_zone(bad), "{bad}");
        }
    }
}
