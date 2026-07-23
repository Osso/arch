#!/bin/sh
set -eu

case "${1:-all}" in
    all)
        cargo fmt --check
        cargo clippy --all-targets --all-features -- -D warnings
        cargo test --all-targets --all-features
        ;;
    unit)
        shift
        cargo test "$@"
        ;;
    *)
        echo "Usage: $0 [all|unit [test-filter]]" >&2
        exit 2
        ;;
esac
