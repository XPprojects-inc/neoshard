# Environment for the Android build (source it): SDK, NDK and JDK paths on this machine.
# Override any of them before sourcing if yours live elsewhere.
: "${ANDROID_HOME:=$LOCALAPPDATA/Android/Sdk}"
if [ -z "${NDK_HOME:-}" ]; then NDK_HOME="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)"; fi
: "${JAVA_HOME:=$HOME/scoop/apps/temurin17-jdk/current}"
export ANDROID_HOME NDK_HOME JAVA_HOME
export PATH="$JAVA_HOME/bin:$PATH"
