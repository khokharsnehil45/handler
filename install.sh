#!/usr/bin/env bash
set -e

# handler installer script
# Usage: curl -sSL https://raw.githubusercontent.com/khokharsnehil45/handler/main/install.sh | bash

REPO="khokharsnehil45/handler"
BINARY_NAME="handler"

BOLD="$(tput bold 2>/dev/null || echo '')"
GREEN="$(tput setaf 2 2>/dev/null || echo '')"
CYAN="$(tput setaf 6 2>/dev/null || echo '')"
YELLOW="$(tput setaf 3 2>/dev/null || echo '')"
RED="$(tput setaf 1 2>/dev/null || echo '')"
RESET="$(tput sgr0 2>/dev/null || echo '')"

echo "${CYAN}${BOLD}⚡ Installing handler - Fast CSV Auditor & Repair Tool...${RESET}"

# 1. Detect OS
OS="$(uname -s)"
case "${OS}" in
    Linux*)     PLATFORM="linux" ;;
    Darwin*)    PLATFORM="macos" ;;
    *)
        echo "${RED}Error: Unsupported operating system: ${OS}${RESET}"
        exit 1
        ;;
esac

# 2. Detect Arch
ARCH="$(uname -m)"
case "${ARCH}" in
    x86_64|amd64) TARGET_ARCH="x86_64" ;;
    aarch64|arm64) TARGET_ARCH="aarch64" ;;
    *)
        echo "${YELLOW}Warning: Architecture '${ARCH}' may not have precompiled releases.${RESET}"
        TARGET_ARCH="${ARCH}"
        ;;
esac

# 3. Determine install destination
if [ -n "${HANDLER_INSTALL_DIR}" ]; then
    INSTALL_DIR="${HANDLER_INSTALL_DIR}"
elif [ -d "${HOME}/.local/bin" ] || mkdir -p "${HOME}/.local/bin" 2>/dev/null; then
    INSTALL_DIR="${HOME}/.local/bin"
elif [ -d "${HOME}/.cargo/bin" ]; then
    INSTALL_DIR="${HOME}/.cargo/bin"
elif [ -w "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
else
    INSTALL_DIR="${HOME}/.local/bin"
fi

mkdir -p "${INSTALL_DIR}"

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

# 4. Attempt to fetch latest release from GitHub
LATEST_TAG="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || echo "")"

DOWNLOADED=0

if [ -n "${LATEST_TAG}" ]; then
    RELEASE_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}/handler-${PLATFORM}-${TARGET_ARCH}.tar.gz"
    echo "Downloading ${CYAN}${LATEST_TAG}${RESET} for ${PLATFORM}-${TARGET_ARCH}..."

    if curl -fsSL "${RELEASE_URL}" -o "${TMP_DIR}/handler.tar.gz" 2>/dev/null; then
        tar -xzf "${TMP_DIR}/handler.tar.gz" -C "${TMP_DIR}"
        if [ -f "${TMP_DIR}/${BINARY_NAME}" ]; then
            DOWNLOADED=1
        fi
    fi
fi

# 5. Fallback: If precompiled release isn't ready or failed, compile via Cargo
if [ "${DOWNLOADED}" -ne 1 ]; then
    if command -v cargo >/dev/null 2>&1; then
        echo "${YELLOW}Prebuilt binary asset not found. Building from source with Cargo...${RESET}"
        cargo install --git "https://github.com/${REPO}.git" --root "${TMP_DIR}/cargo_install"
        if [ -f "${TMP_DIR}/cargo_install/bin/${BINARY_NAME}" ]; then
            cp "${TMP_DIR}/cargo_install/bin/${BINARY_NAME}" "${TMP_DIR}/${BINARY_NAME}"
            DOWNLOADED=1
        fi
    else
        echo "${RED}Error: Precompiled binary for ${PLATFORM}-${TARGET_ARCH} is not yet available, and 'cargo' was not found on your system.${RESET}"
        echo "Please install Rust/Cargo (https://rustup.rs) or download the binary manually."
        exit 1
    fi
fi

# 6. Install binary
chmod +x "${TMP_DIR}/${BINARY_NAME}"
mv "${TMP_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"

echo "${GREEN}${BOLD}✔ Successfully installed handler to: ${INSTALL_DIR}/${BINARY_NAME}${RESET}"

# 7. Check PATH
case ":${PATH}:" in
    *:"${INSTALL_DIR}":*) ;;
    *)
        echo ""
        echo "${YELLOW}Note: '${INSTALL_DIR}' is not in your current PATH.${RESET}"
        echo "Add it by appending this to your ~/.bashrc or ~/.zshrc:"
        echo "    ${BOLD}export PATH=\"${INSTALL_DIR}:\$PATH\"${RESET}"
        ;;
esac

echo ""
echo "${CYAN}Try running:${RESET}"
echo "    ${BOLD}handler --help${RESET}"
echo ""
