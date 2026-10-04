/// Convert one buoy log line to CSV.
pub fn to_csv(line: &str) -> String {
    line.replace(' ', ",")
}
