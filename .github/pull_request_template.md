## Summary

## Changes

## Testing

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo doc --workspace --no-deps --all-features --locked` (no warnings)

## Checklist

- [ ] Changelog updated if user-visible
- [ ] No new `unwrap`/`expect`/`panic`/`todo` (workspace lints)
