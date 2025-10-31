#!/bin/bash
#
# Install git hooks from .githooks directory
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
GIT_HOOKS_DIR="$REPO_ROOT/.git/hooks"

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}Installing git hooks...${NC}"

# Create hooks directory if it doesn't exist
mkdir -p "$GIT_HOOKS_DIR"

# Install all hooks from .githooks directory
for hook in "$SCRIPT_DIR"/*; do
    # Skip this install script and non-executable files
    if [ "$(basename "$hook")" = "install.sh" ] || [ "$(basename "$hook")" = "README.md" ]; then
        continue
    fi

    if [ -f "$hook" ]; then
        hook_name=$(basename "$hook")
        target="$GIT_HOOKS_DIR/$hook_name"

        # Create symlink to the hook
        ln -sf "$SCRIPT_DIR/$hook_name" "$target"
        echo -e "${GREEN}✓ Installed: $hook_name${NC}"
    fi
done

echo -e "${GREEN}✓ Git hooks installed successfully${NC}"
echo ""
echo "Hooks are now active for this repository."
echo "To update hooks in the future, simply run this script again."
