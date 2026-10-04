/// Moon phase index (0..8) for a day number.
pub fn moon_phase(day: u32) -> u32 {
    (day % 30) * 8 / 30
}
