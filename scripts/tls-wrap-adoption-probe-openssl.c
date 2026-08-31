/* plan-110-D Phase 1 spike (Linux): can OpenSSL adopt an already-connected socket
 * without reconnecting?  `SSL_set_fd` is the documented route; this proves it
 * actually handshakes and moves application data over a caller-connected fd.
 *
 *   cc -O0 -o wrap-openssl wrap-openssl.c -ldl
 *
 * Resolves libssl/libcrypto at run time via dlopen, exactly as the shipped tls
 * backend does, so the probe needs no OpenSSL headers on the box.
 */
#include <dlfcn.h>
#include <errno.h>
#include <netdb.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

int main(int argc, char **argv) {
    const char *host = argc > 1 ? argv[1] : "example.com";
    const char *port = argc > 2 ? argv[2] : "443";

    void *ssl_lib = NULL;
    const char *cands[] = {"libssl.so.3", "libssl.so.1.1", "libssl.so"};
    for (unsigned i = 0; i < sizeof(cands) / sizeof(cands[0]); i++) {
        ssl_lib = dlopen(cands[i], RTLD_NOW | RTLD_GLOBAL);
        if (ssl_lib) { printf("dlopen %s OK\n", cands[i]); break; }
    }
    if (!ssl_lib) { printf("no libssl found: %s\n", dlerror()); return 1; }

    void *(*TLS_client_method)(void) = dlsym(ssl_lib, "TLS_client_method");
    void *(*SSL_CTX_new)(const void *) = dlsym(ssl_lib, "SSL_CTX_new");
    void *(*SSL_new)(void *) = dlsym(ssl_lib, "SSL_new");
    int (*SSL_set_fd)(void *, int) = dlsym(ssl_lib, "SSL_set_fd");
    int (*SSL_connect)(void *) = dlsym(ssl_lib, "SSL_connect");
    int (*SSL_write)(void *, const void *, int) = dlsym(ssl_lib, "SSL_write");
    int (*SSL_read)(void *, void *, int) = dlsym(ssl_lib, "SSL_read");
    int (*SSL_get_error)(const void *, int) = dlsym(ssl_lib, "SSL_get_error");
    const char *(*SSL_get_version)(const void *) = dlsym(ssl_lib, "SSL_get_version");
    long (*SSL_ctrl)(void *, int, long, void *) = dlsym(ssl_lib, "SSL_ctrl");
    int (*SSL_set1_host)(void *, const char *) = dlsym(ssl_lib, "SSL_set1_host");
    if (!TLS_client_method || !SSL_CTX_new || !SSL_new || !SSL_set_fd || !SSL_connect) {
        printf("missing symbols\n"); return 1;
    }

    /* 1. Connect an ordinary TCP socket -- what tcp::connect would hand over. */
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port, &hints, &res) != 0) { printf("resolve failed\n"); return 1; }
    int fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (connect(fd, res->ai_addr, res->ai_addrlen) != 0) {
        printf("connect failed errno=%d\n", errno); return 1;
    }
    freeaddrinfo(res);
    printf("plain TCP connected, fd=%d\n", fd);

    /* 2. Adopt that exact fd. */
    void *ctx = SSL_CTX_new(TLS_client_method());
    if (!ctx) { printf("SSL_CTX_new failed\n"); return 1; }
    void *ssl = SSL_new(ctx);
    if (!ssl) { printf("SSL_new failed\n"); return 1; }
    printf("SSL_set_fd(%d) -> %d\n", fd, SSL_set_fd(ssl, fd));
    /* SNI, so the peer serves the right certificate. */
    if (SSL_ctrl) SSL_ctrl(ssl, 55 /* SSL_CTRL_SET_TLSEXT_HOSTNAME */, 0, (void *)host);
    if (SSL_set1_host) SSL_set1_host(ssl, host);

    /* 3. Handshake over the adopted transport. */
    int rc = SSL_connect(ssl);
    printf("SSL_connect    -> %d %s\n", rc, rc == 1 ? "(OK)" : "(FAILED)");
    if (rc != 1) {
        printf("  SSL_get_error = %d\n", SSL_get_error ? SSL_get_error(ssl, rc) : -1);
        close(fd); return 1;
    }
    if (SSL_get_version) printf("negotiated     : %s\n", SSL_get_version(ssl));

    /* 4. Move real application data both ways. */
    char req[256];
    int reqlen = snprintf(req, sizeof(req),
                          "GET / HTTP/1.1\r\nHost: %s\r\nConnection: close\r\n\r\n", host);
    printf("SSL_write      -> %d bytes\n", SSL_write(ssl, req, reqlen));
    char buf[512];
    int got = SSL_read(ssl, buf, sizeof(buf) - 1);
    printf("SSL_read       -> %d bytes\n", got);
    if (got > 0) {
        buf[got] = 0;
        char *nl = strchr(buf, '\r');
        if (nl) *nl = 0;
        printf("first line     : %s\n", buf);
        printf("\nRESULT: an already-connected fd WAS adopted and carried real TLS traffic.\n");
    } else {
        printf("\nRESULT: handshake completed but no application data was read.\n");
    }
    close(fd);
    return 0;
}
