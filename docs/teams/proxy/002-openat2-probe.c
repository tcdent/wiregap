/* openat2 bench probe — sprint 002 slice 1 (zero.proxy-dev, 2026-07-18)
 * Verifies, on the actual bench kernel, each openat2/RESOLVE_* behavior the
 * filesystem-write-path spec's adversarial-closure table relies on.
 * Build: gcc -Wall -O0 -o openat2_probe openat2_probe.c
 * Run inside an empty scratch dir; creates ./probe_base/ fixtures.
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/stat.h>
#include <linux/openat2.h>

static long xopenat2(int dirfd, const char *path, struct open_how *how)
{
    return syscall(SYS_openat2, dirfd, path, how, sizeof(*how));
}

static void probe(const char *label, int dirfd, const char *path,
                  unsigned long long flags, unsigned long long mode,
                  unsigned long long resolve, int expect_ok, int expect_errno)
{
    struct open_how how = { .flags = flags, .mode = mode, .resolve = resolve };
    long fd = xopenat2(dirfd, path, &how);
    int e = (fd < 0) ? errno : 0;
    const char *verdict =
        (expect_ok && fd >= 0) ? "PASS" :
        (!expect_ok && fd < 0 && e == expect_errno) ? "PASS" : "FAIL";
    printf("%-6s %-34s -> %s (fd=%ld errno=%d %s)\n",
           verdict, label, fd >= 0 ? "opened" : "denied", fd,
           e, e ? strerror(e) : "-");
    if (fd >= 0) close(fd);
}

#define R4 (RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | \
            RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV)

int main(void)
{
    /* fixtures */
    (void)!system("rm -rf probe_base && mkdir -p probe_base/real "
                  "&& ln -s /etc probe_base/evil "
                  "&& ln -s real probe_base/rel_link "
                  "&& touch probe_base/real/existing");
    int base = open("probe_base", O_PATH | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    if (base < 0) { perror("base"); return 1; }

    struct utsname { char x[390]; };
    printf("== openat2 RESOLVE probe, resolve=0x%llx ==\n",
           (unsigned long long)R4);

    /* P1: legitimate directory component resolves */
    probe("P1", base, "real", O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0, R4, 1, 0);
    /* P2: symlink as intermediate component (absolute target) */
    probe("P2", base, "evil/passwd", O_RDONLY | O_CLOEXEC, 0, R4, 0, ELOOP);
    /* P3: symlink as final component (relative, in-tree target) */
    probe("P3", base, "rel_link", O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0, R4, 0, ELOOP);
    /* P4: dot-dot escape */
    probe("P4", base, "../..", O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0, R4, 0, EXDEV);
    /* P5: absolute path under RESOLVE_BENEATH */
    probe("P5", base, "/etc", O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0, R4, 0, EXDEV);
    /* P6: O_CREAT|O_EXCL where name exists as symlink */
    probe("P6", base, "evil", O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600, R4, 0, EEXIST);
    /* P7: O_CREAT|O_EXCL where name exists as regular file */
    probe("P7", base, "real/existing", O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600, R4, 0, EEXIST);
    /* P8: O_CREAT|O_EXCL fresh leaf succeeds */
    probe("P8", base, "real/fresh", O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600, R4, 1, 0);
    /* P9: intermediate is a regular file, O_DIRECTORY descent denies */
    probe("P9", base, "real/existing", O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0, R4, 0, ENOTDIR);
    /* P10: magic-link class — via a symlink to /proc/self/fd (belt check) */
    (void)!system("ln -s /proc/self/fd probe_base/maglink 2>/dev/null");
    probe("P10", base, "maglink/0", O_RDONLY | O_CLOEXEC, 0, R4, 0, ELOOP);

    /* P8 side-effect check: created mode */
    struct stat st;
    if (fstatat(base, "real/fresh", &st, AT_SYMLINK_NOFOLLOW) == 0)
        printf("P8b    created mode                       -> %04o (expect 0600 & ~umask)\n",
               st.st_mode & 07777);
    close(base);
    return 0;
}
