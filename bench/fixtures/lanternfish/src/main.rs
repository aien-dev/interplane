mod moon;
mod tides;

fn main() {
    let table = tides::parse_tide_table("data/stations/brest.json");
    println!("lanternfish: {} entries, moon {}", table.len(), moon::moon_phase(0));
}
