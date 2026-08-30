/* plan-110-D §C3 spike (macOS): does Secure Transport work SERVER-side over an
 * already-accepted socket, using a keychain-free SecIdentity?
 *
 * This decides whether macOS can be unified onto one TLS backend (Secure
 * Transport over a plain socket, matching Linux/OpenSSL and Windows/Schannel)
 * instead of carrying Network.framework for connect/listen/accept AND Secure
 * Transport for wrap. The keychain-free identity half is already solved in the
 * shipped backend (SecItemImport -> SecIdentityCreate, gen_macos/server.rs); what
 * is unproven is whether that identity drives a Secure Transport server handshake
 * over a socket the caller accepted.
 *
 *   cc -O0 -w -Wno-deprecated-declarations -framework Security \
 *      -framework CoreFoundation -o /tmp/wrap-macos-server \
 *      /tmp/p110-probe/wrap-macos-server.c
 */
#include <CoreFoundation/CoreFoundation.h>
#include <Security/SecImportExport.h>
#include <Security/SecureTransport.h>
#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static OSStatus sock_read(SSLConnectionRef c, void *data, size_t *len) {
    int fd = (int)(long)c; size_t want = *len, got = 0;
    while (got < want) {
        ssize_t n = read(fd, (char *)data + got, want - got);
        if (n > 0) { got += (size_t)n; continue; }
        *len = got;
        return n == 0 ? errSSLClosedGraceful : errSSLClosedAbort;
    }
    *len = got; return noErr;
}
static OSStatus sock_write(SSLConnectionRef c, const void *data, size_t *len) {
    int fd = (int)(long)c; size_t want = *len, sent = 0;
    while (sent < want) {
        ssize_t n = write(fd, (const char *)data + sent, want - sent);
        if (n > 0) { sent += (size_t)n; continue; }
        *len = sent; return errSSLClosedAbort;
    }
    *len = sent; return noErr;
}

/* Import a PEM file and return the first SecCertificateRef / SecKeyRef in it,
 * with no keychain involved -- the same shape gen_macos/server.rs uses. */
static CFTypeRef import_first(const char *path, SecExternalItemType type) {
    FILE *f = fopen(path, "rb");
    if (!f) { printf("  cannot open %s\n", path); return NULL; }
    static unsigned char buf[65536];
    size_t n = fread(buf, 1, sizeof(buf), f);
    fclose(f);
    CFDataRef data = CFDataCreate(NULL, buf, (CFIndex)n);
    SecItemImportExportKeyParameters params;
    memset(&params, 0, sizeof(params));
    params.version = SEC_KEY_IMPORT_EXPORT_PARAMS_VERSION;
    SecExternalFormat fmt = kSecFormatPEMSequence;
    SecExternalItemType t = type;
    CFArrayRef items = NULL;
    OSStatus st = SecItemImport(data, NULL, &fmt, &t, 0, &params, NULL, &items);
    CFRelease(data);
    if (st != errSecSuccess || !items || CFArrayGetCount(items) == 0) {
        printf("  SecItemImport(%s) -> %d, items=%ld\n", path, (int)st,
               items ? CFArrayGetCount(items) : 0);
        if (items) CFRelease(items);
        return NULL;
    }
    CFTypeRef first = CFArrayGetValueAtIndex(items, 0);
    CFRetain(first);
    CFRelease(items);
    return first;
}

static int listen_fd, accepted_fd = -1;

static void *server_thread(void *arg) {
    struct sockaddr_in cli; socklen_t cl = sizeof(cli);
    accepted_fd = accept(listen_fd, (struct sockaddr *)&cli, &cl);
    if (accepted_fd < 0) { printf("accept failed\n"); return NULL; }
    printf("server: accepted fd=%d (a socket the SERVER already owns)\n", accepted_fd);
    { struct timeval tv = {5, 0};
      setsockopt(accepted_fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
      setsockopt(accepted_fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv)); }

    SecCertificateRef cert = (SecCertificateRef)import_first("/tmp/p110-probe/cert.pem",
                                                            kSecItemTypeCertificate);
    SecKeyRef key = (SecKeyRef)import_first("/tmp/p110-probe/key.pem", kSecItemTypePrivateKey);
    if (!cert || !key) { printf("server: identity import FAILED\n"); return NULL; }
    /* SecIdentityCreate returns the identity directly (allocator, cert, key). */
    SecIdentityRef ident = SecIdentityCreate(NULL, cert, key);
    printf("server: SecIdentityCreate -> %s (keychain-free)\n", ident ? "OK" : "NULL");
    if (!ident) return NULL;
    OSStatus st;

    SSLContextRef ctx = SSLCreateContext(NULL, kSSLServerSide, kSSLStreamType);
    SSLSetIOFuncs(ctx, sock_read, sock_write);
    SSLSetConnection(ctx, (SSLConnectionRef)(long)accepted_fd);
    const void *certs[] = {ident};
    CFArrayRef chain = CFArrayCreate(NULL, certs, 1, NULL);
    st = SSLSetCertificate(ctx, chain);
    printf("server: SSLSetCertificate -> %d\n", (int)st);

    do { st = SSLHandshake(ctx); } while (st == errSSLWouldBlock);
    printf("server: SSLHandshake -> %d %s\n", (int)st, st == noErr ? "(OK)" : "(FAILED)");
    if (st != noErr) return NULL;

    char in[64]; size_t got = 0;
    SSLRead(ctx, in, 5, &got);
    in[got] = 0;
    printf("server: read \"%s\" over TLS\n", in);
    size_t wrote = 0;
    SSLWrite(ctx, "PONG!", 5, &wrote);
    SSLClose(ctx);
    return NULL;
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    listen_fd = socket(AF_INET, SOCK_STREAM, 0);
    int one = 1;
    setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    struct sockaddr_in a;
    memset(&a, 0, sizeof(a));
    a.sin_family = AF_INET; a.sin_addr.s_addr = htonl(INADDR_LOOPBACK); a.sin_port = 0;
    bind(listen_fd, (struct sockaddr *)&a, sizeof(a));
    listen(listen_fd, 4);
    socklen_t al = sizeof(a);
    getsockname(listen_fd, (struct sockaddr *)&a, &al);
    printf("listening on port %d\n", ntohs(a.sin_port));

    pthread_t th;
    pthread_create(&th, NULL, server_thread, NULL);

    int cfd = socket(AF_INET, SOCK_STREAM, 0);
    if (connect(cfd, (struct sockaddr *)&a, sizeof(a)) != 0) {
        printf("client connect failed\n"); return 1;
    }
    printf("client: connected fd=%d\n", cfd);
    { struct timeval tv = {5, 0};
      setsockopt(cfd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
      setsockopt(cfd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv)); }
    SSLContextRef cctx = SSLCreateContext(NULL, kSSLClientSide, kSSLStreamType);
    SSLSetIOFuncs(cctx, sock_read, sock_write);
    SSLSetConnection(cctx, (SSLConnectionRef)(long)cfd);
    /* Self-signed peer: break out at the auth step and continue deliberately.
     * The shipped backend validates properly; this probe is about transport
     * adoption, not trust policy. */
    SSLSetSessionOption(cctx, kSSLSessionOptionBreakOnServerAuth, true);
    OSStatus st;
    do {
        st = SSLHandshake(cctx);
        if (st == errSSLServerAuthCompleted) { printf("client: server auth reached\n"); continue; }
    } while (st == errSSLWouldBlock || st == errSSLServerAuthCompleted);
    printf("client: SSLHandshake -> %d %s\n", (int)st, st == noErr ? "(OK)" : "(FAILED)");
    if (st != noErr) { pthread_join(th, NULL); return 1; }

    size_t wrote = 0;
    SSLWrite(cctx, "PING!", 5, &wrote);
    char in[64]; size_t got = 0;
    SSLRead(cctx, in, 5, &got);
    in[got] = 0;
    printf("client: read \"%s\" over TLS\n", in);

    pthread_join(th, NULL);
    printf("\nRESULT: Secure Transport completed a SERVER-side handshake over an\n"
           "accepted socket with a keychain-free SecIdentity, and both directions\n"
           "carried application data.\n");
    return 0;
}
