#!/bin/sh
# Compile the redsim oracle against the Minecraft server jar in .cache (never committed).
set -e
cd "$(dirname "$0")/.."
MC=.cache/mc/26.3
JAVA_HOME=${JAVA_HOME:-$(ls -d .cache/jdk/*/Contents/Home 2>/dev/null || ls -d .cache/jdk/* | head -1)}
CP="$MC/server-26.3.jar:$(find $MC/bundle/META-INF/libraries -name '*.jar' | tr '\n' ':')"
rm -rf oracle/build && mkdir -p oracle/build
"$JAVA_HOME/bin/javac" -nowarn -encoding UTF-8 --release 25 -cp "$CP" -d oracle/build $(find oracle/src -name '*.java')
echo "$CP" > oracle/build/classpath.txt
echo "oracle built"
