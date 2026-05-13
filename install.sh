#!/usr/bin/env bash
#
# joule-profiler installer
# https://github.com/joule-profiler/joule-profiler
#
set -e

# Configuration
readonly REPO="joule-profiler/joule-profiler"
readonly BINARY_NAME="joule-profiler"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
TARGET_VERSION="${TARGET_VERSION:-latest}"
SKIP_CONFIRM="${SKIP_CONFIRM:-false}"
VERBOSE="${VERBOSE:-false}"

readonly GITHUB_API="https://api.github.com/repos/$REPO"
readonly COLOR_RESET='\033[0m'
readonly COLOR_GREEN='\033[0;32m'
readonly COLOR_RED='\033[0;31m'
readonly COLOR_YELLOW='\033[0;33m'
readonly COLOR_BLUE='\033[0;34m'

# Logging functions
log_info() {
    echo -e "${COLOR_BLUE}ℹ${COLOR_RESET} $*" >&2
}

log_success() {
    echo -e "${COLOR_GREEN}✓${COLOR_RESET} $*" >&2
}

log_warning() {
    echo -e "${COLOR_YELLOW}⚠${COLOR_RESET} $*" >&2
}

log_error() {
    echo -e "${COLOR_RED}✗${COLOR_RESET} $*" >&2
}

log_debug() {
    if [ "$VERBOSE" = true ]; then
        echo -e "${COLOR_YELLOW}[DEBUG]${COLOR_RESET} $*" >&2
    fi
}

# Show help
show_help() {
    cat >&2 << EOF
joule-profiler installer

USAGE:
    install.sh [OPTIONS]

OPTIONS:
    -d, --dir <DIR>         Installation directory (default: /usr/local/bin)
    -v, --version <VER>     Install specific version (default: latest)
    -y, --yes               Skip confirmation prompts
    --verbose               Enable verbose output
    --list                  List available versions
    -h, --help              Show this help message

EXAMPLES:
    # Install latest version to default location
    ./install.sh

    # Install to custom directory
    ./install.sh --dir ~/.local/bin

    # Install specific version
    ./install.sh --version v0.1.0

    # List available versions
    ./install.sh --list

    # Non-interactive installation
    ./install.sh -y

ENVIRONMENT VARIABLES:
    INSTALL_DIR             Installation directory
    TARGET_VERSION          Version to install
    SKIP_CONFIRM            Skip confirmations (true/false)
    VERBOSE                 Verbose output (true/false)

EOF
}

# List available versions
list_available_versions() {
    log_info "Fetching available versions..."

    local versions
    versions=$(curl -fsSL "$GITHUB_API/releases" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' | head -10)

    if [ -z "$versions" ]; then
        log_error "Failed to fetch available versions"
        return 1
    fi

    echo "" >&2
    echo "Available versions:" >&2
    echo "$versions" | while read -r v; do
        echo "  - $v" >&2
    done
    echo "" >&2
}

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -d|--dir)
                INSTALL_DIR="$2"
                shift 2
                ;;
            -v|--version)
                TARGET_VERSION="$2"
                shift 2
                ;;
            -y|--yes)
                SKIP_CONFIRM=true
                shift
                ;;
            --verbose)
                VERBOSE=true
                shift
                ;;
            --list)
                list_available_versions
                exit 0
                ;;
            -h|--help)
                show_help
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                show_help
                exit 1
                ;;
        esac
    done
}

# Check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Check dependencies
check_dependencies() {
    log_debug "Checking dependencies..."
    local missing_deps=()

    for cmd in curl tar sha256sum; do
        if ! command_exists "$cmd"; then
            missing_deps+=("$cmd")
        fi
    done

    if [ ${#missing_deps[@]} -ne 0 ]; then
        log_error "Missing required dependencies: ${missing_deps[*]}"
        log_info "Please install them using your package manager:"
        log_info "  Ubuntu/Debian: sudo apt-get install curl tar coreutils"
        log_info "  Fedora/RHEL: sudo dnf install curl tar coreutils"
        log_info "  Arch: sudo pacman -S curl tar coreutils"
        exit 1
    fi
    log_debug "All dependencies found"
}

# Detect CPU architecture
detect_arch() {
    local arch
    arch=$(uname -m)
    log_debug "Detected architecture: $arch"

    case "$arch" in
        x86_64|amd64)
            echo "x86_64"
            ;;
        *)
            log_error "Unsupported architecture: $arch"
            log_info "joule-profiler only supports x86_64 (Intel/AMD 64-bit)"
            exit 1
            ;;
    esac
}

# Detect system GLIBC version
detect_system_glibc() {
    local version

    # try with ldd --version (always present on GLIBC systems)
    version=$(ldd --version 2>&1 | awk 'NR==1 { print $NF }')
    if [ -n "$version" ]; then
        echo "$version"; return
    fi

    # fallback to libc.so.6 direct execution
    local libc_path
    libc_path=$(find /lib /lib64 /usr/lib 2>/dev/null -name "libc.so.6" | head -1)
    if [ -n "$libc_path" ]; then
        version=$("$libc_path" --version 2>&1 | awk 'NR==1 { print $NF }')
        [ -n "$version" ] && echo "$version" && return
    fi

    echo ""  # not a GLIBC system
}

# Get minimum GLIBC version required by a binary via ELF inspection.
# Returns empty string if objdump/readelf are unavailable.
get_required_glibc() {
    local binary="$1"

    if command_exists objdump; then
        objdump -p "$binary" \
            | grep -oP 'GLIBC_\K[0-9]+\.[0-9]+(\.[0-9]+)?' \
            | sort -V | tail -1
        return
    fi

    if command_exists readelf; then
        readelf -V "$binary" \
            | grep -oP 'GLIBC_\K[0-9]+\.[0-9]+(\.[0-9]+)?' \
            | sort -V | tail -1
        return
    fi

    # No ELF tools available
    echo ""
}

# Returns 0 (true) if $1 >= $2
version_gte() {
    printf '%s\n%s\n' "$2" "$1" | sort -V -C
}

# Tracks whether the GNU binary was already fetched during select_target()
_GNU_PREDOWNLOADED=false

# Determine the best joule-profiler target (GNU or musl) 
select_target() {
    local arch="$1"
    local version="$2"
    local tmp_dir="$3"

    # No ELF tools, using musl
    if ! command_exists objdump && ! command_exists readelf; then
        log_info "No ELF inspection tools found (objdump/readelf) — using musl build"
        echo "${arch}-unknown-linux-musl"
        return
    fi

    # Download GNU binary for inspection
    local gnu_target="${arch}-unknown-linux-gnu"
    local tarball="${BINARY_NAME}-${version}-${gnu_target}.tar.gz"
    local download_url="https://github.com/$REPO/releases/download/${version}/${tarball}"

    log_info "Fetching GNU binary to inspect GLIBC requirements..."
    log_debug "URL: $download_url"

    if ! curl -fsSL -o "$tmp_dir/$tarball" "$download_url" 2>/dev/null; then
        log_warning "Could not fetch GNU binary for inspection — using musl build"
        echo "${arch}-unknown-linux-musl"
        return
    fi

    if ! tar xzf "$tmp_dir/$tarball" -C "$tmp_dir" 2>/dev/null; then
        log_warning "Could not extract GNU binary for inspection — using musl build"
        echo "${arch}-unknown-linux-musl"
        return
    fi

    # Read required GLIBC from ELF
    local required_glibc
    required_glibc=$(get_required_glibc "$tmp_dir/$BINARY_NAME")
    log_debug "Binary requires GLIBC: ${required_GLIBC:-unknown}"

    if [ -z "$required_glibc" ]; then
        log_warning "Could not read GLIBC requirements from binary — using musl build"
        rm -f "$tmp_dir/$BINARY_NAME"
        echo "${arch}-unknown-linux-musl"
        return
    fi

    # Compare with system GLIBC
    local system_glibc
    system_glibc=$(detect_system_glibc)
    log_debug "System GLIBC: ${system_glibc:-not found}"

    if [ -n "$system_glibc" ] && version_gte "$system_glibc" "$required_glibc"; then
        log_success "Compatible GLIBC ($system_glibc >= $required_glibc) — using GNU build"
        _GNU_PREDOWNLOADED=true # binary already in tmp_dir, skip re-download
        echo "$gnu_target"
    else
        if [ -z "$system_glibc" ]; then
            log_info "No GLIBC detected — using musl build"
        else
            log_info "GLIBC too old ($system_glibc < $required_glibc) — using musl build"
        fi

        # discard unusable gnu binary
        rm -f "$tmp_dir/$BINARY_NAME"
        echo "${arch}-unknown-linux-musl"
    fi
}

# Check if running on Linux
check_os() {
    local os
    os=$(uname -s)
    log_debug "Detected OS: $os"

    if [ "$os" != "Linux" ]; then
        log_error "This installer only supports Linux"
        log_info "Detected OS: $os"
        exit 1
    fi
}

# Check Linux distribution
detect_distro() {
    if [ -f /etc/os-release ]; then
        local distro_name distro_version
        distro_name=$(grep '^NAME=' /etc/os-release | cut -d= -f2 | tr -d '"')
        distro_version=$(grep '^VERSION_ID=' /etc/os-release | cut -d= -f2 | tr -d '"')
        log_debug "Distribution: $distro_name $distro_version"
    else
        log_debug "Distribution: Unknown"
    fi
}

# Check if Intel CPU (RAPL requirement)
check_cpu() {
    log_debug "Checking for Intel RAPL support..."

    if [ -d "/sys/class/powercap/intel-rapl" ]; then
        log_success "Intel RAPL detected"

        local domain_count
        domain_count=$(find /sys/class/powercap/intel-rapl -name "intel-rapl:*" -type d 2>/dev/null | wc -l)
        log_debug "Found $domain_count RAPL domain(s)"
    else
        log_warning "Intel RAPL interface not found at /sys/class/powercap/intel-rapl"
        log_warning "joule-profiler requires Intel RAPL support"
        log_warning "Continuing installation, but the tool may not work on this system"
    fi
}

# Get latest release version
get_latest_version() {
    log_info "Fetching latest release..."
    log_debug "GitHub API: $GITHUB_API/releases/latest"

    local version
    version=$(curl -fsSL "$GITHUB_API/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

    if [ -z "$version" ]; then
        log_error "Failed to fetch latest version"
        log_info "Please check your internet connection or try again later"
        log_debug "API endpoint: $GITHUB_API/releases/latest"
        exit 1
    fi

    log_debug "Latest version: $version"
    echo "$version"
}

# Validate version format
validate_version() {
    local version=$1

    if [[ ! $version =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        log_error "Invalid version format: $version"
        log_info "Version must be in format: vX.Y.Z (e.g., v0.1.0)"
        exit 1
    fi
}

# Verify version exists before downloading
verify_version_exists() {
    local version=$1

    log_debug "Checking if version $version exists..."

    local status_code
    status_code=$(curl -o /dev/null -s -w "%{http_code}" "https://github.com/$REPO/releases/tag/$version")

    if [ "$status_code" != "200" ]; then
        log_error "Version $version does not exist"
        log_info ""
        log_info "To see available versions:"
        log_info "  ./install.sh --list"
        log_info ""
        log_info "Or visit: https://github.com/$REPO/releases"
        return 1
    fi

    log_debug "Version $version exists"
    return 0
}

# Download and verify binary
download_binary() {
    local version=$1
    local target=$2 # either x86_64-unknown-linux-gnu or x86_64-unknown-linux-musl
    local tmp_dir=$3

    local tarball="${BINARY_NAME}-${version}-${target}.tar.gz"
    local checksum_file="${tarball}.sha256"
    local download_url="https://github.com/$REPO/releases/download/${version}/${tarball}"
    local checksum_url="https://github.com/$REPO/releases/download/${version}/${checksum_file}"

    log_debug "Target:  $target"
    log_debug "Tarball: $tarball"

    # GNU binary already fetched during select_target()
    if [ "$_GNU_PREDOWNLOADED" = true ] && [ -f "$tmp_dir/$BINARY_NAME" ]; then
        log_debug "Reusing pre-downloaded GNU binary"
    else
        log_info "Downloading joule-profiler ${version} (${target})..."

        local http_code
        http_code=$(curl -fL --progress-bar -w "%{http_code}" -o "$tmp_dir/$tarball" "$download_url" 2>&1 | tail -n1)

        if [ "$http_code" != "200" ]; then
            log_error "Failed to download binary (HTTP $http_code)"

            if [ "$http_code" = "404" ]; then
                log_error "Release $version not found for target $target"
                log_info "Available releases: https://github.com/$REPO/releases"
            else
                log_debug "URL: $download_url"
            fi
            exit 1
        fi

        log_info "Extracting archive..."
        if ! tar xzf "$tmp_dir/$tarball" -C "$tmp_dir" 2>/dev/null; then
            log_error "Failed to extract archive"
            exit 1
        fi

        log_debug "Extraction complete"
    fi

    # Verify checksum
    log_info "Downloading checksum..."
    if ! curl -fsSL -o "$tmp_dir/$checksum_file" "$checksum_url" 2>/dev/null; then
        log_error "Failed to download checksum"
        log_debug "URL: $checksum_url"
        exit 1
    fi

    log_info "Verifying checksum..."
    log_debug "Checksum file: $tmp_dir/$checksum_file"
    cd "$tmp_dir"
    if ! sha256sum -c "$checksum_file" --quiet 2>/dev/null; then
        log_error "Checksum verification failed"
        log_error "The downloaded file may be corrupted or tampered with"
        exit 1
    fi
    cd - > /dev/null

    log_success "Checksum verified"
}

# Install binary
install_binary() {
    local tmp_dir=$1
    local binary_path="$tmp_dir/$BINARY_NAME"

    if [ ! -f "$binary_path" ]; then
        log_error "Binary not found after extraction: $binary_path"
        exit 1
    fi

    log_debug "Binary found: $binary_path"

    # Get binary size
    local binary_size
    if command -v stat >/dev/null 2>&1; then
        binary_size=$(stat -c%s "$binary_path" 2>/dev/null || stat -f%z "$binary_path" 2>/dev/null || echo "unknown")
        log_debug "Binary size: $binary_size bytes"
    fi

    # Create install directory if it doesn't exist
    if [ ! -d "$INSTALL_DIR" ]; then
        log_info "Creating directory $INSTALL_DIR..."
        if ! mkdir -p "$INSTALL_DIR" 2>/dev/null; then
            if command_exists sudo; then
                sudo mkdir -p "$INSTALL_DIR"
            else
                log_error "Cannot create directory $INSTALL_DIR"
                exit 1
            fi
        fi
    fi

    log_info "Installing to $INSTALL_DIR..."

    # Check if we need sudo
    if [ -w "$INSTALL_DIR" ]; then
        log_debug "Installing without sudo (writable directory)"
        install -m 755 "$binary_path" "$INSTALL_DIR/$BINARY_NAME"
    else
        if ! command_exists sudo; then
            log_error "sudo is required to install to $INSTALL_DIR"
            log_info "Please run this script as root or install sudo"
            exit 1
        fi
        log_debug "Installing with sudo (non-writable directory)"
        sudo install -m 755 "$binary_path" "$INSTALL_DIR/$BINARY_NAME"
    fi

    log_success "Installed $BINARY_NAME to $INSTALL_DIR"
}

# Check if already installed
check_existing_installation() {
    if command_exists "$BINARY_NAME"; then
        local installed_version
        installed_version=$("$BINARY_NAME" --version 2>/dev/null | grep -oP '\d+\.\d+\.\d+' || echo "unknown")
        local installed_path
        installed_path=$(command -v "$BINARY_NAME")

        log_warning "$BINARY_NAME is already installed"
        log_info "  Version: $installed_version"
        log_info "  Location: $installed_path"

        if [ "$SKIP_CONFIRM" = false ]; then
            echo -n "Do you want to overwrite it? [y/N] " >&2
            read -r reply </dev/tty
            
            if [[ ! $reply =~ ^[Yy]$ ]]; then
                log_info "Installation cancelled"
                exit 0
            fi
        else
            log_info "Overwriting (--yes flag enabled)"
        fi
    fi
}

# Check if install directory is in PATH
check_path() {
    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        log_warning "$INSTALL_DIR is not in your PATH"
        log_info "Add it to your PATH by adding this line to your shell profile:"

        local shell_profile=""
        if [ -n "$BASH_VERSION" ]; then
            shell_profile="~/.bashrc"
        elif [ -n "$ZSH_VERSION" ]; then
            shell_profile="~/.zshrc"
        else
            shell_profile="~/.profile"
        fi

        log_info "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> $shell_profile"
        log_info "  source $shell_profile"
    fi
}

# Print usage information
print_usage() {
    echo "" >&2
    echo -e "${COLOR_GREEN}Installation complete!${COLOR_RESET}" >&2
    echo "" >&2
    echo "To get started with Joule Profiler:" >&2
    echo "" >&2
    echo "  # List available sensors" >&2
    echo -e "  ${COLOR_BLUE}joule-profiler list-sensors${COLOR_RESET}" >&2
    echo "" >&2
    echo "  # Phase-based profiling" >&2
    echo -e "  ${COLOR_BLUE}sudo joule-profiler profile -- <your-command>${COLOR_RESET}" >&2
    echo "" >&2
    echo "  # Show help" >&2
    echo -e "  ${COLOR_BLUE}joule-profiler --help${COLOR_RESET}" >&2
    echo "" >&2
    echo "If you want to launch Joule Profiler without root privileges (root), configure the perf_event_paranoid level:" >&2
    echo -e "  ${COLOR_BLUE}sudo sysctl kernel.perf_event_paranoid=0${COLOR_RESET}" >&2
    echo "" >&2
    echo "For more information, visit:" >&2
    echo "  https://github.com/$REPO" >&2
    echo "" >&2
}

# Main installation
main() {
    parse_args "$@"

    echo "" >&2
    echo "╔════════════════════════════════════════════╗" >&2
    echo "║          Joule Profiler installer          ║" >&2
    echo "║  github.com/joule-profiler/joule-profiler  ║" >&2
    echo "╚════════════════════════════════════════════╝" >&2
    echo "" >&2

    log_debug "Installation directory: $INSTALL_DIR"
    log_debug "Target version: $TARGET_VERSION"
    log_debug "Skip confirm: $SKIP_CONFIRM"
    log_debug "Verbose mode: $VERBOSE"

    # Pre-flight checks
    check_os
    detect_distro
    check_dependencies
    check_existing_installation

    local arch
    arch=$(detect_arch)
    log_success "Detected architecture: $arch"

    check_cpu

    local version
    if [ "$TARGET_VERSION" = "latest" ]; then
        version=$(get_latest_version)
    else
        version="$TARGET_VERSION"
        validate_version "$version"
        log_info "Installing specific version: $version"
        if ! verify_version_exists "$version"; then
            exit 1
        fi
    fi
    log_success "Target version: $version"

    # Create temporary directory
    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "$tmp_dir"' EXIT
    log_debug "Temporary directory: $tmp_dir"

    # Select the best target for this machine
    local target
    target=$(select_target "$arch" "$version" "$tmp_dir")
    log_success "Selected target: $target"

    # Download, verify checksum, and install
    download_binary "$version" "$target" "$tmp_dir"
    install_binary "$tmp_dir"

    # Verify installation
    if ! command_exists "$BINARY_NAME"; then
        log_error "Installation failed: $BINARY_NAME not found in PATH"
        log_info "Make sure $INSTALL_DIR is in your PATH"
        exit 1
    fi

    local installed_version
    installed_version=$("$BINARY_NAME" --version 2>/dev/null | grep -oP '\d+\.\d+\.\d+' || echo "unknown")
    log_success "Verified installation: $BINARY_NAME $installed_version"

    check_path
    print_usage
}

main "$@"
