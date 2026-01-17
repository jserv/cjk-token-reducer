#!/bin/bash

# This management script supports install, uninstall, and status check.

set -e

# ANSI color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Print colored message
print_info()
{
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warn()
{
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error()
{
    echo -e "${RED}[ERROR]${NC} $1"
}

print_status()
{
    echo -e "${BLUE}[STATUS]${NC} $1"
}

# Print usage information
print_usage()
{
    echo "Usage: $0 [install|uninstall|status]"
    echo ""
    echo "Commands:"
    echo "  install   - Install cjk-token-reducer and configure Claude Code hooks"
    echo "  uninstall - Remove cjk-token-reducer and Claude Code hooks"
    echo "  status    - Show installation status"
    echo "  help      - Show this help message"
}

# Detect OS and determine install directory
detect_install_dir()
{
    local install_dir=""

    case "$(uname -s)" in
        Linux | Darwin)
            # Use XDG compliant directory or fallback to ~/.local/bin
            if [ -n "$XDG_BIN_HOME" ]; then
                install_dir="$XDG_BIN_HOME"
            elif [ -d "$HOME/.local/bin" ]; then
                install_dir="$HOME/.local/bin"
            else
                install_dir="$HOME/.local/bin"
            fi
            ;;
        MINGW* | MSYS* | CYGWIN*)
            # Windows: use ~/.local/bin or AppData/Local
            if [ -d "$HOME/.local/bin" ]; then
                install_dir="$HOME/.local/bin"
            else
                install_dir="$APPDATA/cjk-token-reducer"
            fi
            ;;
        *)
            print_error "Unsupported operating system: $(uname -s)"
            exit 1
            ;;
    esac

    echo "$install_dir"
}

# Check if Claude CLI is installed
check_claude_cli()
{
    if ! command -v claude &> /dev/null; then
        print_warn "Claude CLI not found in PATH"
        echo ""
        echo "Please install Claude Code first:"
        echo "  npm install -g @anthropic-ai/claude-code"
        echo ""
        return 1
    fi

    print_info "Claude CLI found: $(which claude)"
    return 0
}

# Ensure install directory exists
ensure_install_dir()
{
    local install_dir="$1"

    if [ ! -d "$install_dir" ]; then
        print_info "Creating install directory: $install_dir"
        mkdir -p "$install_dir"
    fi
}

# Copy binary to install directory
install_binary()
{
    local install_dir="$1"
    local binary_path="target/release/cjk-token-reducer"

    if [ ! -f "$binary_path" ]; then
        print_error "Binary not found: $binary_path"
        echo ""
        echo "Please build the binary first:"
        echo "  cargo build --release"
        echo "  # On macOS with NLP support: cargo build --release --features macos-nlp"
        echo ""
        exit 1
    fi

    print_info "Installing binary to: $install_dir/cjk-token-reducer"
    cp "$binary_path" "$install_dir/cjk-token-reducer"
    chmod +x "$install_dir/cjk-token-reducer"
}

# Verify binary is accessible after installation
verify_installation()
{
    local install_dir="$1"

    if [ ! -x "$install_dir/cjk-token-reducer" ]; then
        print_error "Binary installation failed or is not executable"
        exit 1
    fi

    print_info "Binary installed successfully"
}

# Check if PATH includes install directory
check_path()
{
    local install_dir="$1"

    # Remove trailing slash for comparison
    install_dir="${install_dir%/}"

    case ":$PATH:" in
        *":$install_dir:"*)
            print_info "Install directory is in PATH"
            return 0
            ;;
        *)
            print_warn "Install directory is not in PATH"
            echo ""
            echo "Add the following to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
            echo "  export PATH=\"$install_dir:\$PATH\""
            echo ""
            return 1
            ;;
    esac
}

# Check if hook is already configured
hook_exists()
{
    local settings_file="$1"
    local binary_path="${2:-cjk-token-reducer}" # Use provided path or default to command name

    if [ ! -f "$settings_file" ]; then
        return 1
    fi

    # Check if cjk-token-reducer is already in the hooks (either command name or absolute path)
    if grep -q "cjk-token-reducer" "$settings_file" 2> /dev/null \
        && (grep -q "\"$binary_path\"" "$settings_file" 2> /dev/null); then
        return 0
    fi

    return 1
}

# Validate JSON file
validate_json()
{
    local json_file="$1"

    if ! command -v python3 &> /dev/null; then
        print_warn "python3 not found, skipping JSON validation"
        return 0
    fi

    if ! python3 -m json.tool "$json_file" > /dev/null 2>&1; then
        return 1
    fi

    return 0
}

# Configure Claude Code hook
configure_hook()
{
    local claude_dir="$HOME/.claude"
    local settings_file="$claude_dir/settings.json"
    local backup_file="$claude_dir/settings.json.backup"

    # Create .claude directory if it doesn't exist
    if [ ! -d "$claude_dir" ]; then
        print_info "Creating Claude config directory: $claude_dir"
        mkdir -p "$claude_dir"
    fi

    # Check if hook is already configured
    if hook_exists "$settings_file" "$binary_path"; then
        print_info "Hook is already configured in $settings_file"
        return 0
    fi

    # Create backup if settings.json exists
    if [ -f "$settings_file" ]; then
        print_info "Creating backup of existing settings.json"
        cp "$settings_file" "$backup_file"
    fi

    # Determine the absolute path of the cjk-token-reducer binary
    local install_dir
    install_dir=$(detect_install_dir)
    local binary_path="$install_dir/cjk-token-reducer"

    # Prepare hook configuration to be added
    local new_hook
    new_hook="{
      \"hooks\": [
        {
          \"type\": \"command\",
          \"command\": \"$binary_path\"
        }
      ]
    }"

    # If settings.json doesn't exist, create it with the hook
    if [ ! -f "$settings_file" ]; then
        print_info "Creating Claude settings.json with hook configuration"
        echo '{ "hooks": { "UserPromptSubmit": [] } }' > "$settings_file"
    fi

    # Use python to add the hook to existing configuration
    if ! command -v python3 &> /dev/null; then
        print_error "python3 is required to modify JSON configuration"
        print_info "Manual addition required: Add cjk-token-reducer hook to $settings_file"
        exit 1
    fi

    local modified_json
    modified_json=$(python3 -c "
import json
import sys

try:
    with open('$settings_file', 'r') as f:
        settings = json.load(f)

    # Ensure hooks section exists
    if 'hooks' not in settings:
        settings['hooks'] = {}

    # Ensure UserPromptSubmit section exists
    if 'UserPromptSubmit' not in settings['hooks']:
        settings['hooks']['UserPromptSubmit'] = []

    # Add the new hook
    new_hook = $new_hook
    settings['hooks']['UserPromptSubmit'].append(new_hook)

    print(json.dumps(settings, indent=2))
except Exception as e:
    sys.stderr.write(f'Error modifying JSON: {e}\n')
    sys.exit(1)
" 2>&1)

    if [ $? -ne 0 ]; then
        print_error "Failed to modify JSON configuration"
        print_error "$modified_json"
        print_info "Restoring backup from: $backup_file"
        [ -f "$backup_file" ] && cp "$backup_file" "$settings_file"
        exit 1
    fi

    echo "$modified_json" > "$settings_file"

    print_info "Hook configured successfully"
}

# Remove binary
remove_binary()
{
    local install_dir="$1"
    local binary_path="$install_dir/cjk-token-reducer"

    if [ ! -f "$binary_path" ]; then
        print_warn "Binary not found at: $binary_path"
        return 0
    fi

    print_info "Removing binary: $binary_path"
    rm -f "$binary_path"
    print_info "Binary removed successfully"
}

# Remove hook from Claude settings
remove_hook()
{
    local claude_dir="$HOME/.claude"
    local settings_file="$claude_dir/settings.json"
    local backup_file="$claude_dir/settings.json.before-uninstall"

    if [ ! -f "$settings_file" ]; then
        print_warn "Claude settings file not found: $settings_file"
        return 0
    fi

    # Determine the absolute path of the cjk-token-reducer binary
    local install_dir
    install_dir=$(detect_install_dir)
    local binary_path="$install_dir/cjk-token-reducer"

    # Check if hook exists (checking for both command name and absolute path)
    if ! grep -q "cjk-token-reducer" "$settings_file" 2> /dev/null; then
        print_info "Hook not found in settings.json"
        return 0
    fi

    print_info "Removing hook from $settings_file"

    # Create backup before modification
    cp "$settings_file" "$backup_file"

    # Use python to remove the hook
    if ! command -v python3 &> /dev/null; then
        print_error "python3 is required to modify JSON configuration"
        print_info "Manual removal required: Edit $settings_file and remove cjk-token-reducer from hooks"
        exit 1
    fi

    local modified_json
    modified_json=$(python3 -c "
import json
import sys

try:
    with open('$settings_file', 'r') as f:
        settings = json.load(f)

    # Remove cjk-token-reducer hooks (checking for both command name and absolute path)
    if 'hooks' in settings and 'UserPromptSubmit' in settings['hooks']:
        original_hooks = settings['hooks']['UserPromptSubmit']
        filtered_hooks = []

        for entry in original_hooks:
            if isinstance(entry, dict) and 'hooks' in entry:
                # Filter out cjk-token-reducer command hooks (both command name and absolute path)
                filtered_entry_hooks = [
                    h for h in entry['hooks']
                    if not (h.get('command') == 'cjk-token-reducer' or h.get('command') == '$binary_path')
                ]

                # Keep entry if it has other hooks
                if filtered_entry_hooks:
                    entry['hooks'] = filtered_entry_hooks
                    filtered_hooks.append(entry)
            else:
                # Keep non-hook entries
                filtered_hooks.append(entry)

        settings['hooks']['UserPromptSubmit'] = filtered_hooks

        # Remove empty UserPromptSubmit array
        if not settings['hooks']['UserPromptSubmit']:
            del settings['hooks']['UserPromptSubmit']

        # Remove empty hooks object
        if not settings['hooks']:
            del settings['hooks']

    print(json.dumps(settings, indent=2))
except Exception as e:
    sys.stderr.write(f'Error modifying JSON: {e}\n')
    sys.exit(1)
" 2>&1)

    if [ $? -ne 0 ]; then
        print_error "Failed to modify JSON configuration"
        print_error "$modified_json"
        print_info "Backup saved to: $backup_file"
        exit 1
    fi

    echo "$modified_json" > "$settings_file"
    print_info "Hook removed successfully"
    print_info "Backup saved to: $backup_file"
}

# Check installation status
check_status()
{
    local install_dir
    install_dir=$(detect_install_dir)
    local binary_path="$install_dir/cjk-token-reducer"

    print_status "Checking installation status..."

    # Check if binary exists
    if [ -f "$install_dir/cjk-token-reducer" ]; then
        print_info "Binary is installed at: $install_dir/cjk-token-reducer"
    else
        print_warn "Binary is not installed"
    fi

    # Check if Claude CLI is available
    if command -v claude &> /dev/null; then
        print_info "Claude CLI is available: $(which claude)"
    else
        print_warn "Claude CLI is not installed"
    fi

    # Check if PATH includes install directory
    check_path "$install_dir"

    # Check Claude settings
    local claude_dir="$HOME/.claude"
    local settings_file="$claude_dir/settings.json"

    if [ -f "$settings_file" ] && hook_exists "$settings_file" "$binary_path"; then
        print_info "Claude hook is configured in: $settings_file"
    else
        print_warn "Claude hook is not configured"
    fi
}

# Main installation flow
perform_install()
{
    echo "=========================================="
    echo "  cjk-token-reducer Installer"
    echo "=========================================="
    echo ""

    # Check if Claude CLI is installed
    check_claude_cli || exit 1

    # Detect install directory
    local install_dir
    install_dir=$(detect_install_dir)

    # Ensure install directory exists
    ensure_install_dir "$install_dir"

    # Install binary
    install_binary "$install_dir"

    # Verify installation
    verify_installation "$install_dir"

    # Check PATH
    check_path "$install_dir"

    # Configure Claude hook
    configure_hook

    echo ""
    echo "=========================================="
    echo "  Installation Complete"
    echo "=========================================="
    echo ""
    echo "Binary installed to: $install_dir/cjk-token-reducer"
    echo "Claude hook configured in: ~/.claude/settings.json"
    echo ""
    echo "You can now use cjk-token-reducer with Claude Code!"
    echo ""
}

# Main uninstallation flow
perform_uninstall()
{
    echo "=========================================="
    echo "  cjk-token-reducer Uninstaller"
    echo "=========================================="
    echo ""

    # Confirm uninstallation
    read -p "Are you sure you want to uninstall cjk-token-reducer? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Uninstallation cancelled"
        exit 0
    fi
    echo

    # Detect install directory
    local install_dir
    install_dir=$(detect_install_dir)

    # Remove hook first
    remove_hook

    # Remove binary
    remove_binary "$install_dir"

    echo ""
    echo "=========================================="
    echo "  Uninstallation Complete"
    echo "=========================================="
    echo ""
    echo "Binary removed from: $install_dir/cjk-token-reducer"
    echo "Hook removed from: ~/.claude/settings.json"
    echo ""
    echo "To completely remove cached data:"
    case "$(uname -s)" in
        Darwin)
            echo "  rm -rf ~/Library/Caches/cjk-token-reducer"
            echo "  rm -rf ~/Library/Application\\ Support/cjk-token-reducer"
            ;;
        Linux)
            echo "  rm -rf ~/.cache/cjk-token-reducer"
            echo "  rm -rf ~/.config/cjk-token-reducer"
            ;;
        MINGW* | MSYS* | CYGWIN*)
            echo "  rm -rf %LOCALAPPDATA%\\cjk-token-reducer"
            echo "  rm -rf %APPDATA%\\cjk-token-reducer"
            ;;
    esac
    echo ""
}

# Parse command line arguments
case "${1:-help}" in
    install)
        perform_install
        ;;
    uninstall)
        perform_uninstall
        ;;
    status)
        check_status
        ;;
    help | "")
        print_usage
        ;;
    *)
        print_error "Unknown command: $1"
        echo ""
        print_usage
        exit 1
        ;;
esac
