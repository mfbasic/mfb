/* ICMP capability probe (plan-110-A Phase 1).
 *
 * Establishes, per OS, the facts `net::ping`'s native backends are built on.  The
 * matrix in `planning/plan-110-A-network-contract-and-ping.md` §Corrections C1 was
 * produced by running this file; re-run it to re-derive that matrix rather than
 * trusting the table.
 *
 * POSIX only (macOS + Linux).  Windows uses `iphlpapi`'s Icmp* API instead of a
 * socket and is probed by inspecting `IPHLPAPI.DLL`'s exports; see C1.
 *
 * Build and run:
 *     cc -O0 -w -o /tmp/icmp-probe scripts/icmp-capability-probe.c
 *     /tmp/icmp-probe                 # local + default-route probes
 *     /tmp/icmp-probe 1.1.1.1         # override the off-link target
 *
 * What each section answers:
 *   [perm]   Is SOCK_DGRAM/IPPROTO_ICMP available unprivileged?  Is SOCK_RAW?
 *   [shape]  Does a reply arrive with the IPv4 header attached (macOS) or as a
 *            bare ICMP message (Linux)?  Where does the reply TTL come from?
 *   [id]     Does the kernel rewrite the echo identifier?  (Linux yes, macOS no.)
 *   [demux]  Does a second ICMP socket also receive our reply?  (macOS yes.)
 *   [size]   Largest payload `sendto` accepts.
 *   [err]    Are Time Exceeded / Destination Unreachable delivered, and can they
 *            be matched back to our echo through the quoted original header?
 */

#include <errno.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#ifndef IP_RECVTTL
#define IP_RECVTTL 12
#endif
/* The cmsg_type a Linux IP_RECVTTL control message actually arrives with is
 * IP_TTL (2), NOT the IP_RECVTTL (12) used to enable it. */
#define CMSG_TYPE_IP_TTL 2

#define ICMP_ECHO_REQUEST 8
#define ICMP_ECHO_REPLY 0
#define ICMP_DEST_UNREACH 3
#define ICMP_TIME_EXCEEDED 11

static unsigned short cksum(unsigned short *b, int len) {
    unsigned long s = 0;
    while (len > 1) {
        s += *b++;
        len -= 2;
    }
    if (len == 1) s += *(unsigned char *)b;
    s = (s >> 16) + (s & 0xffff);
    s += (s >> 16);
    return (unsigned short)(~s);
}

static long long now_us(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000LL + t.tv_nsec / 1000;
}

/* Fill an echo request of `payload` bytes with id/seq and a valid checksum. */
static int build_echo(unsigned char *pkt, int payload, unsigned short id, unsigned short seq) {
    int len = 8 + payload;
    memset(pkt, 0, len);
    pkt[0] = ICMP_ECHO_REQUEST;
    pkt[4] = (unsigned char)(id >> 8);
    pkt[5] = (unsigned char)(id & 0xff);
    pkt[6] = (unsigned char)(seq >> 8);
    pkt[7] = (unsigned char)(seq & 0xff);
    for (int i = 0; i < payload; i++) pkt[8 + i] = (unsigned char)(i & 0xff);
    unsigned short ck = cksum((unsigned short *)pkt, len);
    memcpy(pkt + 2, &ck, 2);
    return len;
}

static int open_icmp(void) { return socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP); }

static void section(const char *tag) { printf("\n=== [%s] ===\n", tag); }

/* ---- [perm] ------------------------------------------------------------- */
static int probe_permission(void) {
    section("perm");
    int d = open_icmp();
    printf("SOCK_DGRAM/IPPROTO_ICMP fd=%d errno=%d (%s)\n", d, d < 0 ? errno : 0,
           d < 0 ? strerror(errno) : "-");
    int r = socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
    printf("SOCK_RAW /IPPROTO_ICMP fd=%d errno=%d (%s)\n", r, r < 0 ? errno : 0,
           r < 0 ? strerror(errno) : "-");
    if (r >= 0) close(r);
    if (d < 0) {
        printf("ICMP is DENIED for this user -- this is a valid permission-denial "
               "environment for the plan-110-A Phase 3 test.\n");
        return 0;
    }
    close(d);
    return 1;
}

/* ---- [shape] [id] [err] -------------------------------------------------- */
static void probe_echo(const char *target, int payload, int ttl, int timeout_ms,
                       unsigned short id, unsigned short seq) {
    printf("-- target=%s payload=%d ttl=%d timeout=%dms id=%04x seq=%u\n", target, payload, ttl,
           timeout_ms, id, seq);
    int fd = open_icmp();
    if (fd < 0) {
        printf("   socket denied errno=%d (%s)\n", errno, strerror(errno));
        return;
    }
    setsockopt(fd, IPPROTO_IP, IP_TTL, &ttl, sizeof(ttl));
    int on = 1;
    int recvttl = setsockopt(fd, IPPROTO_IP, IP_RECVTTL, &on, sizeof(on));

    unsigned char *pkt = calloc(1, 70000);
    int len = build_echo(pkt, payload, id, seq);

    struct sockaddr_in dst;
    memset(&dst, 0, sizeof(dst));
    dst.sin_family = AF_INET;
    if (inet_pton(AF_INET, target, &dst.sin_addr) != 1) {
        printf("   bad target\n");
        goto out;
    }

    long long t0 = now_us();
    if (sendto(fd, pkt, len, 0, (struct sockaddr *)&dst, sizeof(dst)) < 0) {
        printf("   sendto errno=%d (%s)\n", errno, strerror(errno));
        goto out;
    }

    for (;;) {
        int remain = timeout_ms - (int)((now_us() - t0) / 1000);
        if (remain < 0) remain = 0;
        struct pollfd p = {fd, POLLIN, 0};
        int r = poll(&p, 1, remain);
        if (r == 0) {
            printf("   TIMEOUT after %lld us\n", now_us() - t0);
            break;
        }
        if (r < 0) {
            if (errno == EINTR) continue;
            printf("   poll errno=%d\n", errno);
            break;
        }

        unsigned char buf[70000];
        unsigned char cbuf[512];
        struct iovec iov = {buf, sizeof(buf)};
        struct sockaddr_in from;
        struct msghdr msg;
        memset(&msg, 0, sizeof(msg));
        msg.msg_name = &from;
        msg.msg_namelen = sizeof(from);
        msg.msg_iov = &iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf;
        msg.msg_controllen = sizeof(cbuf);
        ssize_t n = recvmsg(fd, &msg, 0);
        if (n < 0) {
            printf("   recvmsg errno=%d (%s)\n", errno, strerror(errno));
            break;
        }
        long long rtt = now_us() - t0;

        int hlen = ((buf[0] >> 4) == 4) ? (buf[0] & 0x0f) * 4 : 0;
        int ip_ttl = hlen ? buf[8] : -1;
        int cmsg_ttl = -1;
        for (struct cmsghdr *c = CMSG_FIRSTHDR(&msg); c; c = CMSG_NXTHDR(&msg, c))
            if (c->cmsg_level == IPPROTO_IP &&
                (c->cmsg_type == CMSG_TYPE_IP_TTL || c->cmsg_type == IP_RECVTTL))
                memcpy(&cmsg_ttl, CMSG_DATA(c), sizeof(int));

        unsigned char *ic = buf + hlen;
        printf("   recv %zd from=%s | iphdr=%s ip_ttl=%d | IP_RECVTTL setsockopt=%d cmsg_ttl=%d | "
               "type=%d code=%d id=%04x seq=%04x | rtt=%lld us\n",
               n, inet_ntoa(from.sin_addr), hlen ? "PRESENT" : "absent", ip_ttl, recvttl, cmsg_ttl,
               ic[0], ic[1], (ic[4] << 8) | ic[5], (ic[6] << 8) | ic[7], rtt);
        if (ic[0] == ICMP_ECHO_REPLY)
            printf("      id %s (sent %04x)\n",
                   ((ic[4] << 8) | ic[5]) == id ? "PRESERVED" : "REWRITTEN BY KERNEL", id);
        if (ic[0] == ICMP_TIME_EXCEEDED || ic[0] == ICMP_DEST_UNREACH) {
            unsigned char *orig = ic + 8; /* quoted original IP header + 8 bytes */
            int ohl = (orig[0] & 0x0f) * 4;
            unsigned char *oic = orig + ohl;
            printf("      quoted original: proto=%d type=%d id=%04x seq=%04x -> matches ours: %s\n",
                   orig[9], oic[0], (oic[4] << 8) | oic[5], (oic[6] << 8) | oic[7],
                   ((oic[6] << 8) | oic[7]) == seq ? "YES" : "no");
        }
        break;
    }
out:
    free(pkt);
    close(fd);
}

/* ---- [demux] ------------------------------------------------------------ */
static void probe_demux(void) {
    section("demux");
    int a = open_icmp(), b = open_icmp();
    if (a < 0 || b < 0) {
        printf("skipped (ICMP denied)\n");
        if (a >= 0) close(a);
        if (b >= 0) close(b);
        return;
    }
    unsigned char pkt[64];
    int len = build_echo(pkt, 56, 0x5A5A, 99);
    struct sockaddr_in dst;
    memset(&dst, 0, sizeof(dst));
    dst.sin_family = AF_INET;
    inet_pton(AF_INET, "127.0.0.1", &dst.sin_addr);
    if (sendto(a, pkt, len, 0, (struct sockaddr *)&dst, sizeof(dst)) < 0) {
        printf("sendto errno=%d\n", errno);
        goto out;
    }
    usleep(300000);
    struct pollfd pa = {a, POLLIN, 0}, pb = {b, POLLIN, 0};
    int ra = poll(&pa, 1, 0), rb = poll(&pb, 1, 0);
    printf("sent on socket a: a readable=%d b readable=%d -> %s\n", ra, rb,
           rb > 0 ? "PROMISCUOUS (reply match MUST check id+seq)"
                  : "per-socket demux (type+seq match is sufficient)");
out:
    close(a);
    close(b);
}

/* ---- [size] ------------------------------------------------------------- */
static int try_send(int payload) {
    int fd = open_icmp();
    if (fd < 0) return -1;
    unsigned char *pkt = calloc(1, 80000);
    int len = build_echo(pkt, payload, 0xABCD, 1);
    struct sockaddr_in dst;
    memset(&dst, 0, sizeof(dst));
    dst.sin_family = AF_INET;
    inet_pton(AF_INET, "127.0.0.1", &dst.sin_addr);
    ssize_t n = sendto(fd, pkt, len, 0, (struct sockaddr *)&dst, sizeof(dst));
    int e = n < 0 ? errno : 0;
    free(pkt);
    close(fd);
    return e;
}

static void probe_max_payload(void) {
    section("size");
    if (try_send(0) != 0) {
        printf("skipped (ICMP denied)\n");
        return;
    }
    int lo = 0, hi = 70000;
    while (lo < hi) {
        int mid = (lo + hi + 1) / 2;
        if (try_send(mid) == 0)
            lo = mid;
        else
            hi = mid - 1;
    }
    printf("max payload accepted by sendto = %d (ICMP message %d, IP total %d); "
           "payload %d -> errno %d\n",
           lo, lo + 8, lo + 28, lo + 1, try_send(lo + 1));
}

int main(int argc, char **argv) {
    const char *off_link = argc > 1 ? argv[1] : "8.8.8.8";

    if (!probe_permission()) return 0; /* denial is itself a complete result */

    section("shape / id");
    probe_echo("127.0.0.1", 0, 64, 2000, 0xABCD, 1);
    probe_echo("127.0.0.1", 56, 64, 2000, 0xABCD, 2);
    probe_echo("127.0.0.1", 1472, 64, 2000, 0xABCD, 3);

    probe_demux();
    probe_max_payload();

    section("err");
    printf("silent address (TEST-NET-1) -> expect a clean deadline expiry:\n");
    probe_echo("192.0.2.1", 56, 64, 1500, 0xABCD, 6);
    printf("ttl=1 off-link -> expect ICMP Time Exceeded (a NAT that re-originates "
           "the echo will answer normally instead):\n");
    probe_echo(off_link, 56, 1, 3000, 0xABCD, 7);
    printf("normal off-link echo:\n");
    probe_echo(off_link, 56, 64, 3000, 0xABCD, 8);
    return 0;
}
