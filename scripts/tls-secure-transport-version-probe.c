/* Does Secure Transport support TLS 1.3?  If it caps at 1.2, unifying macOS onto
 * it (plan-110-D C3) would DOWNGRADE macOS TLS from what Network.framework
 * negotiates today, which is a security regression and an argument against the
 * unification.
 *
 * Asks for TLS 1.3 explicitly via SSLSetProtocolVersionMax and reports what was
 * actually negotiated against a host known to offer 1.3.
 */
#include <Security/SecureTransport.h>
#include <errno.h>
#include <netdb.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static OSStatus rd(SSLConnectionRef c, void *d, size_t *l) {
    int fd = (int)(long)c; size_t want = *l, got = 0;
    while (got < want) { ssize_t n = read(fd, (char *)d + got, want - got);
        if (n > 0) { got += (size_t)n; continue; } *l = got;
        return n == 0 ? errSSLClosedGraceful : errSSLClosedAbort; }
    *l = got; return noErr;
}
static OSStatus wr(SSLConnectionRef c, const void *d, size_t *l) {
    int fd = (int)(long)c; size_t want = *l, sent = 0;
    while (sent < want) { ssize_t n = write(fd, (const char *)d + sent, want - sent);
        if (n > 0) { sent += (size_t)n; continue; } *l = sent; return errSSLClosedAbort; }
    *l = sent; return noErr;
}

static const char *name_of(SSLProtocol p) {
    switch ((int)p) {
        case 0: return "unknown"; case 2: return "SSL 3.0"; case 4: return "TLS 1.0";
        case 7: return "TLS 1.1"; case 8: return "TLS 1.2"; case 10: return "TLS 1.3";
        default: return "other";
    }
}

int main(int argc, char **argv) {
    const char *host = argc > 1 ? argv[1] : "example.com";
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, "443", &hints, &res)) { printf("resolve failed\n"); return 1; }
    int fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (connect(fd, res->ai_addr, res->ai_addrlen)) { printf("connect failed\n"); return 1; }
    freeaddrinfo(res);

    SSLContextRef ctx = SSLCreateContext(NULL, kSSLClientSide, kSSLStreamType);
    SSLSetIOFuncs(ctx, rd, wr);
    SSLSetConnection(ctx, (SSLConnectionRef)(long)fd);
    SSLSetPeerDomainName(ctx, host, strlen(host));

    /* Explicitly ask for the highest we can name. kTLSProtocol13 == 10. */
    OSStatus st = SSLSetProtocolVersionMax(ctx, (SSLProtocol)10);
    printf("SSLSetProtocolVersionMax(TLS 1.3) -> %d %s\n", (int)st,
           st == noErr ? "(accepted)" : "(REJECTED -- 1.3 not expressible)");
    SSLProtocol asked = kSSLProtocolUnknown;
    SSLGetProtocolVersionMax(ctx, &asked);
    printf("SSLGetProtocolVersionMax          -> %d (%s)\n", (int)asked, name_of(asked));

    do { st = SSLHandshake(ctx); } while (st == errSSLWouldBlock);
    printf("SSLHandshake                      -> %d\n", (int)st);
    SSLProtocol got = kSSLProtocolUnknown;
    SSLGetNegotiatedProtocolVersion(ctx, &got);
    printf("negotiated                        -> %d (%s)\n", (int)got, name_of(got));
    printf("\n%s\n", (int)got == 10
        ? "RESULT: Secure Transport negotiated TLS 1.3 -- no downgrade."
        : "RESULT: Secure Transport capped BELOW TLS 1.3 against a 1.3-capable peer.");
    SSLClose(ctx); close(fd);
    return 0;
}
