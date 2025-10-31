# Git Hooks for SyndDB

This directory contains git hooks that are tracked in version control and shared across the team.

## Installation

To install the git hooks, run:

```bash
./.githooks/install.sh
```

Or using the make command (if available):

```bash
make install-hooks
```

This creates symlinks from `.git/hooks/` to the hooks in this directory, so updates to hooks are automatically picked up.

## Available Hooks

### pre-commit

Runs before each commit to ensure code quality:

- **Rust formatting**: Runs `cargo fmt` on all staged `.rs` files
- **Solidity formatting**: Runs `forge fmt` on all staged `.sol` files in the `contracts/` directory

If any files are reformatted, they are automatically re-staged for commit.

### commit-msg

Validates commit messages follow the [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

**Valid types**: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `perf`, `ci`, `build`, `revert`

**Examples**:
- `feat: add user authentication`
- `fix(api): handle null pointer exception`
- `docs: update README with installation steps`
- `chore!: drop support for Node 12` (breaking change)

## Requirements

- **Rust**: Install from https://rustup.rs/
- **Foundry** (for Solidity): Install from https://book.getfoundry.sh/getting-started/installation

## Updating Hooks

When hooks are updated in this directory:

1. The changes are automatically picked up (symlinks)
2. Team members pull the changes with `git pull`
3. The updated hooks run automatically on their next commit

## Bypassing Hooks

In rare cases where you need to bypass hooks (use sparingly):

```bash
git commit --no-verify
```

## Troubleshooting

**Hook not running?**
- Ensure hooks are installed: `./.githooks/install.sh`
- Check that hooks are executable: `ls -la .githooks/`

**Permission denied?**
```bash
chmod +x .githooks/*
```

**Hook failing?**
- Ensure you have the required tools installed (Rust, Foundry)
- Check the error message for specific issues
- Manually run the formatter to see detailed output:
  - Rust: `cargo fmt`
  - Solidity: `cd contracts && forge fmt`
