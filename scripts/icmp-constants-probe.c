/* ICMP / IP socket constant and struct-layout probe (plan-110-A Phase 2).
 *
 * The `net::ping` backends hardcode numeric socket-option values, clock ids, and
 * `msghdr`/`cmsghdr` field offsets into emitted machine code, where a wrong value
 * is silent — the call simply fails, or reads the wrong bytes, at run time on a
 * platform the host cannot execute.  This prints every such number from the
 * platform's own headers so the values committed to
 * `CodegenPlatform` are transcribed from a measurement rather than recalled.
 *
 *     cc -O0 -o /tmp/icmp-consts scripts/icmp-constants-probe.c && /tmp/icmp-consts
 *
 * Run it on each supported POSIX target and compare against the table in
 * `planning/plan-110-A-network-contract-and-ping.md` §Corrections C5.
 */

#include <netinet/in.h>
#include <stddef.h>
#include <stdio.h>
#include <sys/socket.h>
#include <time.h>

int main(void) {
#if defined(__APPLE__)
    printf("platform: macOS\n");
#elif defined(__linux__)
    printf("platform: Linux\n");
#else
    printf("platform: other\n");
#endif
    printf("\n-- socket / protocol --\n");
    printf("AF_INET          = %d\n", AF_INET);
    printf("SOCK_DGRAM       = %d\n", SOCK_DGRAM);
    printf("SOCK_RAW         = %d\n", SOCK_RAW);
    printf("IPPROTO_ICMP     = %d\n", IPPROTO_ICMP);
    printf("IPPROTO_IP       = %d\n", IPPROTO_IP);

    printf("\n-- socket-level setsockopt options --\n");
    printf("SOL_SOCKET       = %d\n", SOL_SOCKET);
    printf("SO_RCVBUF        = %d\n", SO_RCVBUF);

    printf("\n-- IP-level setsockopt options --\n");
    printf("IP_TTL           = %d\n", IP_TTL);
#ifdef IP_RECVTTL
    printf("IP_RECVTTL       = %d\n", IP_RECVTTL);
#else
    printf("IP_RECVTTL       = <undefined>\n");
#endif

    printf("\n-- clocks --\n");
    printf("CLOCK_MONOTONIC  = %d\n", (int)CLOCK_MONOTONIC);
    printf("CLOCK_REALTIME   = %d\n", (int)CLOCK_REALTIME);

    printf("\n-- struct msghdr layout (recvmsg) --\n");
    printf("sizeof(struct msghdr)     = %zu\n", sizeof(struct msghdr));
    printf("  msg_name       offset %2zu size %zu\n", offsetof(struct msghdr, msg_name),
           sizeof(((struct msghdr *)0)->msg_name));
    printf("  msg_namelen    offset %2zu size %zu\n", offsetof(struct msghdr, msg_namelen),
           sizeof(((struct msghdr *)0)->msg_namelen));
    printf("  msg_iov        offset %2zu size %zu\n", offsetof(struct msghdr, msg_iov),
           sizeof(((struct msghdr *)0)->msg_iov));
    printf("  msg_iovlen     offset %2zu size %zu\n", offsetof(struct msghdr, msg_iovlen),
           sizeof(((struct msghdr *)0)->msg_iovlen));
    printf("  msg_control    offset %2zu size %zu\n", offsetof(struct msghdr, msg_control),
           sizeof(((struct msghdr *)0)->msg_control));
    printf("  msg_controllen offset %2zu size %zu\n", offsetof(struct msghdr, msg_controllen),
           sizeof(((struct msghdr *)0)->msg_controllen));
    printf("  msg_flags      offset %2zu size %zu\n", offsetof(struct msghdr, msg_flags),
           sizeof(((struct msghdr *)0)->msg_flags));

    printf("\n-- struct iovec layout --\n");
    printf("sizeof(struct iovec)      = %zu\n", sizeof(struct iovec));
    printf("  iov_base       offset %2zu\n", offsetof(struct iovec, iov_base));
    printf("  iov_len        offset %2zu size %zu\n", offsetof(struct iovec, iov_len),
           sizeof(((struct iovec *)0)->iov_len));

    printf("\n-- struct cmsghdr layout (control message) --\n");
    printf("sizeof(struct cmsghdr)    = %zu\n", sizeof(struct cmsghdr));
    printf("  cmsg_len       offset %2zu size %zu\n", offsetof(struct cmsghdr, cmsg_len),
           sizeof(((struct cmsghdr *)0)->cmsg_len));
    printf("  cmsg_level     offset %2zu size %zu\n", offsetof(struct cmsghdr, cmsg_level),
           sizeof(((struct cmsghdr *)0)->cmsg_level));
    printf("  cmsg_type      offset %2zu size %zu\n", offsetof(struct cmsghdr, cmsg_type),
           sizeof(((struct cmsghdr *)0)->cmsg_type));
    {
        /* CMSG_DATA's offset from the cmsghdr base, and the padded length of a
         * one-int control message -- both are macro-defined and differ by libc. */
        char buf[128];
        struct cmsghdr *c = (struct cmsghdr *)buf;
        printf("  CMSG_DATA(c) - c          = %td\n", (unsigned char *)CMSG_DATA(c) - (unsigned char *)c);
        printf("  CMSG_LEN(sizeof(int))     = %u\n", (unsigned)CMSG_LEN(sizeof(int)));
        printf("  CMSG_SPACE(sizeof(int))   = %u\n", (unsigned)CMSG_SPACE(sizeof(int)));
    }

    printf("\n-- struct sockaddr_in layout --\n");
    printf("sizeof(struct sockaddr_in) = %zu\n", sizeof(struct sockaddr_in));
    printf("  sin_family     offset %2zu size %zu\n", offsetof(struct sockaddr_in, sin_family),
           sizeof(((struct sockaddr_in *)0)->sin_family));
    printf("  sin_port       offset %2zu\n", offsetof(struct sockaddr_in, sin_port));
    printf("  sin_addr       offset %2zu\n", offsetof(struct sockaddr_in, sin_addr));
    return 0;
}
