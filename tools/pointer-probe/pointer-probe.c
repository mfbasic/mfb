/* pointer-probe -- put the real pointer somewhere, for a test of a real window.
 *
 *   pointer-probe window <pid>   print "x y width height" of <pid>'s frontmost
 *                                on-screen window, in global display points
 *                                (origin top-left), or exit 1 if it has none.
 *   pointer-probe move <x> <y>   move the pointer to global point (x, y) and post
 *                                the mouse-moved event a human hand would have made.
 *   pointer-probe down <x> <y>   press the left button at (x, y);
 *   pointer-probe drag <x> <y>   move to (x, y) with the left button held;
 *   pointer-probe up <x> <y>     release it there.
 *   pointer-probe where          print the pointer's "x y", in the same coordinates.
 *   pointer-probe trusted        print 1 if this process may post events, else 0.
 *
 * macOS only. See README.md beside this file for why it exists and what it needs.
 */
#include <ApplicationServices/ApplicationServices.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int window_of(pid_t pid) {
    CFArrayRef list = CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID);
    if (!list) return 1;
    int found = 1;
    /* The list is front to back, so the first match is the frontmost. */
    for (CFIndex i = 0; i < CFArrayGetCount(list); i++) {
        CFDictionaryRef info = CFArrayGetValueAtIndex(list, i);
        CFNumberRef owner = CFDictionaryGetValue(info, kCGWindowOwnerPID);
        CFNumberRef layer = CFDictionaryGetValue(info, kCGWindowLayer);
        int owner_pid = 0, layer_number = 0;
        if (!owner || !CFNumberGetValue(owner, kCFNumberIntType, &owner_pid)) continue;
        if (layer) CFNumberGetValue(layer, kCFNumberIntType, &layer_number);
        if (owner_pid != pid || layer_number != 0) continue;
        CGRect bounds;
        CFDictionaryRef rect = CFDictionaryGetValue(info, kCGWindowBounds);
        if (!rect || !CGRectMakeWithDictionaryRepresentation(rect, &bounds)) continue;
        printf("%d %d %d %d\n", (int)bounds.origin.x, (int)bounds.origin.y,
               (int)bounds.size.width, (int)bounds.size.height);
        found = 0;
        break;
    }
    CFRelease(list);
    return found;
}

static int move_to(double x, double y) {
    CGPoint at = CGPointMake(x, y);
    /* The warp moves the cursor whether or not this process may post events; the
     * posted event is what makes AppKit deliver mouseMoved: to the view under it. */
    CGWarpMouseCursorPosition(at);
    CGEventRef event = CGEventCreateMouseEvent(NULL, kCGEventMouseMoved, at, kCGMouseButtonLeft);
    if (!event) return 1;
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event);
    return 0;
}

static int button(CGEventType type, double x, double y) {
    CGPoint at = CGPointMake(x, y);
    CGEventRef event = CGEventCreateMouseEvent(NULL, type, at, kCGMouseButtonLeft);
    if (!event) return 1;
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event);
    return 0;
}

int main(int argc, char **argv) {
    if (argc == 4 && strcmp(argv[1], "down") == 0)
        return button(kCGEventLeftMouseDown, atof(argv[2]), atof(argv[3]));
    if (argc == 4 && strcmp(argv[1], "drag") == 0)
        return button(kCGEventLeftMouseDragged, atof(argv[2]), atof(argv[3]));
    if (argc == 4 && strcmp(argv[1], "up") == 0)
        return button(kCGEventLeftMouseUp, atof(argv[2]), atof(argv[3]));
    if (argc == 3 && strcmp(argv[1], "window") == 0) return window_of((pid_t)atoi(argv[2]));
    if (argc == 4 && strcmp(argv[1], "move") == 0) return move_to(atof(argv[2]), atof(argv[3]));
    if (argc == 2 && strcmp(argv[1], "where") == 0) {
        CGEventRef now = CGEventCreate(NULL);
        if (!now) return 1;
        CGPoint at = CGEventGetLocation(now);
        CFRelease(now);
        printf("%d %d\n", (int)at.x, (int)at.y);
        return 0;
    }
    if (argc == 2 && strcmp(argv[1], "trusted") == 0) {
        printf("%d\n", AXIsProcessTrusted() ? 1 : 0);
        return 0;
    }
    fprintf(stderr, "usage: pointer-probe window <pid> | move|down|drag|up <x> <y> | where | trusted\n");
    return 2;
}
