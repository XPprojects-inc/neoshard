# Source this in Git Bash on Windows without Visual Studio Build Tools:
#   . scripts/env.sh && cargo test
# Uses the windows-gnu Rust toolchain with MSYS2 UCRT64 gcc as linker/C compiler.
# On Linux/macOS this file is a no-op.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    MSYS2_ROOT="${MSYS2_ROOT:-$HOME/scoop/apps/msys2/current}"
    export PATH="$HOME/.cargo/bin:$MSYS2_ROOT/ucrt64/bin:$PATH"
    export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable-x86_64-pc-windows-gnu}"
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
    ;;
esac
