#!/bin/bash
#
# Price Oracle Development Environment
#
# This is a thin wrapper around the justfile. Run from project root:
#   just example-price-oracle
#
# Or run this script directly:
#   ./scripts/dev-env.sh
#
# For more options, see: just --list

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

cd "$PROJECT_ROOT"

# Check if just is installed
if ! command -v just &> /dev/null; then
    echo "Error: 'just' is not installed."
    echo "Install with: brew install just  OR  cargo install just"
    exit 1
fi

exec just examples::price-oracle
