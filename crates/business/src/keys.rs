//! `keys.ts`: the edition (`NEXT_PUBLIC_EDITION`, default `community`).

/// `NEXT_PUBLIC_EDITION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    Community,
    Cloud,
    Enterprise,
}

impl Edition {
    /// `NEXT_PUBLIC_EDITION` of the process (anything unknown = community).
    pub fn from_env() -> Edition {
        Edition::parse(std::env::var("NEXT_PUBLIC_EDITION").ok().as_deref())
    }

    /// The edition of a `NEXT_PUBLIC_EDITION` value.
    pub fn parse(value: Option<&str>) -> Edition {
        match value {
            Some("cloud") => Edition::Cloud,
            Some("enterprise") => Edition::Enterprise,
            _ => Edition::Community,
        }
    }

    /// `isCommunity()`.
    pub fn is_community(self) -> bool {
        self == Edition::Community
    }

    /// `isCloud()`.
    pub fn is_cloud(self) -> bool {
        self == Edition::Cloud
    }

    /// The `NEXT_PUBLIC_EDITION` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Edition::Community => "community",
            Edition::Cloud => "cloud",
            Edition::Enterprise => "enterprise",
        }
    }
}
