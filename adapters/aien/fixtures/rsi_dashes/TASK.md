# Remove the banned dashes from the README

task_id: vac/m5-rsi-dashes-1
test_command: cargo test --offline --locked -q
target_file: README.md

The README uses em and en dashes, which the house style bans. The RSI engine proposes the
replacement; the independent judge evaluates it on holdouts the proposer cannot read.
