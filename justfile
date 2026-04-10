# ai-wiki justfile

# Default: check, test, lint
default: check test lint

# Fast compile check (no codegen)
check:
    cargo check --workspace

# Run all tests
test:
    cargo test --workspace

# Run tests with output
test-verbose:
    cargo test --workspace -- --nocapture

# Run clippy lints
lint:
    cargo clippy --workspace

# Format code
fmt:
    cargo fmt

# Check formatting without applying
fmt-check:
    cargo fmt --check

# Build debug binaries
build:
    cargo build --workspace

# Build optimized release binaries
release:
    cargo build --release

# Run the full CI pipeline (check, test, lint, fmt)
ci: check test lint fmt-check

# Ingest files into the queue
ingest path:
    cargo run -p ai-wiki -- ingest "{{path}}"

# Ingest from a file list
ingest-list listfile:
    cargo run -p ai-wiki -- ingest "@{{listfile}}"

# Process all queued items using Claude
process:
    cargo run -p ai-wiki -- process

# Launch the TUI monitor
tui:
    cargo run -p ai-wiki -- tui

# Start the MCP server (for claude mcp add)
mcp:
    cargo run -p ai-wiki-mcp

# Register the MCP server with Claude Code
mcp-register:
    claude mcp add ai-wiki -- cargo run --manifest-path {{justfile_directory()}}/Cargo.toml -p ai-wiki-mcp

# Install ai-wiki and ai-wiki-mcp to ~/.cargo/bin
deploy:
    cargo install --path crates/ai-wiki
    cargo install --path crates/ai-wiki-mcp

# Clean build artifacts
clean:
    cargo clean

# Show binary sizes (release)
sizes: release
    @ls -lh target/release/ai-wiki target/release/ai-wiki-mcp

# Run a single crate's tests
test-core:
    cargo test -p ai-wiki-core

test-cli:
    cargo test -p ai-wiki

test-mcp:
    cargo test -p ai-wiki-mcp

# Count lines of code
loc:
    @find crates -name '*.rs' | xargs wc -l | tail -1

# Show test count
test-count:
    @cargo test --workspace 2>&1 | grep "test result" | grep -oP '\d+ passed' | awk '{s += $1} END {print s " tests total"}'
