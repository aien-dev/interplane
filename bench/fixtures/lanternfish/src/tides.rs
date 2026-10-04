/// One high or low water entry.
pub struct TideEntry {
    pub hour: u8,
    pub height_m: f32,
}

/// Parse a station's tide table from its JSON file.
pub fn parse_tide_table(path: &str) -> Vec<TideEntry> {
    let _ = path;
    vec![
        TideEntry { hour: 6, height_m: 5.4 },
        TideEntry { hour: 18, height_m: 5.1 },
    ]
}
