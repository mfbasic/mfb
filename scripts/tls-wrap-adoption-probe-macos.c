/* plan-110-D Phase 1 spike (macOS): can an ALREADY-CONNECTED socket be adopted by
 * a TLS implementation on macOS, without reconnecting?
 *
 * The shipped `tls::connect` backend is Network.framework, which owns its socket
 * end to end and exposes no raw fd — so it cannot wrap one. The candidate route is
 * Secure Transport (SSLCreateContext + SSLSetIOFuncs + SSLSetConnection), the API
 * that exists precisely to run TLS over caller-supplied I/O. It is deprecated but
 * present. This probe proves whether it actually completes a handshake and moves
 * application data over an fd the caller connected itself.
 *
 *   cc -O0 -Wno-deprecated-declarations -framework Security -framework CoreFoundation \
 *      -o /tmp/wrap-macos /tmp/p110-probe/wrap-macos.c
 */
#include <Security/SecureTransport.h>
#include <arpa/inet.h>
#include <errno.h>
#include <netdb.h>
#include <netinet/in.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

/* Secure Transport calls these with whatever "connection" we registered; ours is
 * the fd itself. This is the whole adoption mechanism: TLS never learns what the
 * transport is. */
static OSStatus sock_read(SSLConnectionRef conn, void *data, size_t *len) {
    int fd = (int)(long)conn;
    size_t want = *len, got = 0;
    while (got < want) {
        ssize_t n = read(fd, (char *)data + got, want - got);
        if (n > 0) { got += (size_t)n; continue; }
        *len = got;
        if (n == 0) return errSSLClosedGraceful;
        if (errno == EAGAIN || errno == EWOULDBLOCK) return errSSLWouldBlock;
        return errSSLClosedAbort;
    }
    *len = got;
    return noErr;
}

static OSStatus sock_write(SSLConnectionRef conn, const void *data, size_t *len) {
    int fd = (int)(long)conn;
    size_t want = *len, sent = 0;
    while (sent < want) {
        ssize_t n = write(fd, (const char *)data + sent, want - sent);
        if (n > 0) { sent += (size_t)n; continue; }
        *len = sent;
        if (errno == EAGAIN || errno == EWOULDBLOCK) return errSSLWouldBlock;
        return errSSLClosedAbort;
    }
    *len = sent;
    return noErr;
}

int main(int argc, char **argv) {
    const char *host = argc > 1 ? argv[1] : "example.com";
    const char *port = argc > 2 ? argv[2] : "443";

    /* 1. Connect an ordinary TCP socket -- this is what tcp::connect hands over. */
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port, &hints, &res) != 0) { printf("resolve failed\n"); return 1; }
    int fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (fd < 0) { printf("socket failed\n"); return 1; }
    if (connect(fd, res->ai_addr, res->ai_addrlen) != 0) {
        printf("connect failed errno=%d\n", errno); return 1;
    }
    freeaddrinfo(res);
    printf("plain TCP connected, fd=%d\n", fd);

    /* 2. Adopt that exact fd. No reconnect happens anywhere below. */
    SSLContextRef ctx = SSLCreateContext(NULL, kSSLClientSide, kSSLStreamType);
    if (!ctx) { printf("SSLCreateContext failed\n"); return 1; }
    OSStatus st = SSLSetIOFuncs(ctx, sock_read, sock_write);
    printf("SSLSetIOFuncs   -> %d\n", (int)st);
    st = SSLSetConnection(ctx, (SSLConnectionRef)(long)fd);
    printf("SSLSetConnection-> %d  (the fd IS the connection ref)\n", (int)st);
    st = SSLSetPeerDomainName(ctx, host, strlen(host));
    printf("SSLSetPeerDomainName(%s) -> %d\n", host, (int)st);

    /* 3. Handshake over the adopted transport. */
    do { st = SSLHandshake(ctx); } while (st == errSSLWouldBlock);
    printf("SSLHandshake    -> %d %s\n", (int)st, st == noErr ? "(OK)" : "(FAILED)");
    if (st != noErr) { close(fd); return 1; }

    SSLProtocol proto = kSSLProtocolUnknown;
    SSLGetNegotiatedProtocolVersion(ctx, &proto);
    printf("negotiated protocol enum = %d\n", (int)proto);

    /* 4. Move real application data both ways. */
    char req[256];
    int reqlen = snprintf(req, sizeof(req),
                          "GET / HTTP/1.1\r\nHost: %s\r\nConnection: close\r\n\r\n", host);
    size_t wrote = 0;
    st = SSLWrite(ctx, req, (size_t)reqlen, &wrote);
    printf("SSLWrite        -> %d, %zu bytes\n", (int)st, wrote);

    char buf[512];
    size_t got = 0;
    st = SSLRead(ctx, buf, sizeof(buf) - 1, &got);
    buf[got < sizeof(buf) ? got : sizeof(buf) - 1] = 0;
    printf("SSLRead         -> %d, %zu bytes\n", (int)st, got);
    if (got > 0) {
        char *nl = strchr(buf, '\r');
        if (nl) *nl = 0;
        printf("first line      : %s\n", buf);
        printf("\nRESULT: an already-connected fd WAS adopted and carried real TLS traffic.\n");
    } else {
        printf("\nRESULT: handshake completed but no application data was read.\n");
    }

    SSLClose(ctx);
    CFRelease(ctx);
    close(fd);
    return 0;
}
