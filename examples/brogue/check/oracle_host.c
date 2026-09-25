// oracle_host: the globals oracle/src/platform/main.c defines, for drivers that
// link the whole engine but bring their own main(). Values match main.c's
// defaults for a non-interactive, text-mode run.

#include <stdio.h>
#include <string.h>
#include "platform.h"

struct brogueConsole currentConsole;
boolean serverMode = false;
boolean nonInteractivePlayback = false;
boolean hasGraphics = false;
enum graphicsModes graphicsMode = TEXT_GRAPHICS;

// Copied from oracle/src/platform/main.c; the engine calls it when parsing
// seeds typed into the menu.
boolean tryParseUint64(char *str, uint64_t *num) {
    unsigned long long n;
    char buf[100];
    if (strlen(str)
        && sscanf(str, "%llu", &n)
        && sprintf(buf, "%llu", n)
        && !strcmp(buf, str)) {
        *num = (uint64_t)n;
        return true;
    } else {
        return false;
    }
}
