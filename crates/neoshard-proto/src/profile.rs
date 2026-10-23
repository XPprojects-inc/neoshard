//! Catalog of profile cosmetics (ADR-0020). The server validates ids against
//! these lists; clients render them (NeoShard's own SVG/CSS art, no third-party
//! assets). Forks may extend the lists: unknown ids are rejected on write and
//! ignored on render.

/// Frames drawn around the avatar.
pub const AVATAR_DECORATIONS: &[&str] = &[
    "orbit",
    "petals",
    "frost",
    "flame",
    "circuit",
    "stardust",
    "leaves",
    "crown",
    "neon",
    "shards",
    "cat_ears",
    "headphones",
    "halo",
    "hearts",
];

/// Animated overlays on the profile card (disabled with reduced motion).
pub const PROFILE_EFFECTS: &[&str] = &[
    "snowfall",
    "sparkles",
    "aurora",
    "bubbles",
    "embers",
    "rain",
    "confetti",
    "starfield",
];

/// Backgrounds behind the name in member and DM lists.
pub const NAMEPLATES: &[&str] = &[
    "dusk", "lagoon", "ember", "forest", "nebula", "paper", "grid", "waves",
];

/// Display-name fonts (system font stacks, nothing is downloaded).
pub const NAME_FONTS: &[&str] = &["sans", "serif", "mono", "rounded", "slab", "hand"];

/// Display-name effects.
pub const NAME_EFFECTS: &[&str] = &["solid", "gradient", "neon", "toon", "pop", "shimmer"];

/// Badges next to a server tag.
pub const TAG_BADGES: &[&str] = &[
    "shard", "star", "leaf", "flame", "wave", "bolt", "heart", "moon",
];

/// Limits shared by the server and clients.
pub const BIO_MAX: usize = 190;
pub const PRONOUNS_MAX: usize = 40;
pub const DISPLAY_NAME_MAX: usize = 32;
pub const CUSTOM_STATUS_MAX: usize = 128;
pub const NOTE_MAX: usize = 256;
pub const LINKS_MAX: usize = 6;

/// `""` (none) or a known id.
pub fn valid(list: &[&str], id: &str) -> bool {
    id.is_empty() || list.contains(&id)
}

/// 1–4 letters or digits (any script).
pub fn valid_tag(tag: &str) -> bool {
    let n = tag.chars().count();
    (1..=4).contains(&n) && tag.chars().all(char::is_alphanumeric)
}

/// Colours are 24-bit RGB.
pub fn valid_color(c: u32) -> bool {
    c <= 0xFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog() {
        assert!(valid(AVATAR_DECORATIONS, ""));
        assert!(valid(AVATAR_DECORATIONS, "orbit"));
        assert!(!valid(AVATAR_DECORATIONS, "../x"));
        assert!(valid_tag("NS"));
        assert!(valid_tag("ОСК1"));
        assert!(!valid_tag(""));
        assert!(!valid_tag("TOOLONG"));
        assert!(!valid_tag("a b"));
        assert!(valid_color(0xFFFFFF));
        assert!(!valid_color(0x1000000));
    }
}
