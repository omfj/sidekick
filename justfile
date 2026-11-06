# Install sidekick locally on your machine.
install:
    cargo install --path .

# Check formatting and linting issues.
check:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings

# Fix formatting and linting issues.
fix:
    cargo fmt --all
    cargo clippy --all-targets --all-features --fix --allow-dirty --allow-staged
