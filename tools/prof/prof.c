// LD_PRELOAD wall-clock sampling profiler + file-event timeline for pdftex.
// Build: gcc -O2 -shared -fPIC -o prof.so prof.c -ldl -lrt
// Env: PROF_OUT=<file>  PROF_T0=<CLOCK_MONOTONIC ns at spawn>  PROF_US=<sample interval us, 0=off>
#define _GNU_SOURCE
#include <dlfcn.h>
#include <execinfo.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <time.h>
#include <ucontext.h>
#include <unistd.h>
#include <dirent.h>
#include <link.h>
#include <sys/syscall.h>
#include <linux/perf_event.h>
#include <linux/hw_breakpoint.h>
#include <sys/ioctl.h>

typedef uint64_t u64;
#define MAXFRAMES 48
#define NEV 400000
#define NSAMP 60000
#define MAXTEX 96

typedef struct { u64 t; int kind; int pathoff; long a; long b; } ev_t;
typedef struct { u64 t; u64 rip; int n; int ntex; u64 fr[MAXFRAMES]; int tex[MAXTEX]; } samp_t;

static ev_t *evs; static volatile int nev;
static samp_t *samps; static volatile int nsamp;
static char *pathbuf; static int pathoff;
static u64 t0, t_ctor, t_main;
static timer_t tid; static int tid_ok;
static int active;
typedef struct { unsigned char state, index; long start, loc, limit, name, tag; } instate;
static instate *p_curinput; static instate **p_inputstack; static long *p_inputptr;
typedef struct { long lh, rh; } twoh; static twoh **p_hash; static unsigned char **p_strpool; static long **p_strstart;
static u64 exe_base;
static u64 t_dump_end;
// perf counters
#define NPERF 16
static int pfd[NPERF]; static const char *pname[NPERF]; static int npf;
static long long pv_main[NPERF];
static int bpfd[4]; static const char *bpname[4]; static int nbp;

// accumulators [count, total ns]
enum { C_READ, C_WRITE, C_FREAD, C_FWRITE, C_FFLUSH, C_INFLATE, C_DEFLATE, C_GZREAD, C_STAT, C_ACCESS, C_OPENDIR, C_FOPEN, C_OPEN, C_MMAP, C_MALLOC, C_GETC, C__N };
static const char *cname[] = {"read","write","fread","fwrite","fflush","inflate","deflate","gzread","stat","access","opendir","fopen","open","mmap","malloc","getc"};
static u64 ccnt[C__N], ctot[C__N], cbytes[C__N];

static inline u64 now(void) { struct timespec ts; clock_gettime(CLOCK_MONOTONIC, &ts); return ts.tv_sec * 1000000000ull + ts.tv_nsec; }

static int addpath(const char *p) {
  if (!p || !pathbuf) return -1;
  int l = strlen(p); if (pathoff + l + 1 > (1 << 24)) return -1;
  memcpy(pathbuf + pathoff, p, l + 1); int o = pathoff; pathoff += l + 1; return o;
}
static long long (*pq)[2];
static void addev(int kind, const char *p, long a, long b) {
  if (!evs) return; int i = nev; if (i >= NEV) return; nev = i + 1;
  evs[i].t = now(); evs[i].kind = kind; evs[i].pathoff = addpath(p); evs[i].a = a; evs[i].b = b;
  if (npf >= 2 && (kind == 10 || kind == 21 || kind == 2)) { extern ssize_t __read(int, void *, size_t); long long v[2] = {0, 0}; for (int q = 0; q < 2; q++) { long r = syscall(SYS_read, pfd[q], &v[q], sizeof(long long)); (void)r; } pq[i][0] = v[0]; pq[i][1] = v[1]; }
}

static void on_prof(int sig, siginfo_t *si, void *uc_) {
  (void)sig; (void)si;
  int save = 0; extern int *__errno_location(void); save = *__errno_location();
  ucontext_t *uc = uc_;
  int i = nsamp;
  if (i < NSAMP && active) {
    nsamp = i + 1;
    samp_t *s = &samps[i];
    s->t = now(); s->rip = uc->uc_mcontext.gregs[REG_RIP];
    void *bt[MAXFRAMES + 4];
    int n = backtrace(bt, MAXFRAMES + 4);
    // skip handler and restorer frames: bt[0]=on_prof, bt[1]=__restore_rt, bt[2]=interrupted rip
    int k = 0; for (int j = 2; j < n && k < MAXFRAMES; j++) s->fr[k++] = (u64)bt[j];
    s->n = k;
    s->ntex = 0;
    if (p_curinput && *p_inputstack) {
      long ip = *p_inputptr; int nt = 0;
      // innermost first: current then stack downward; macro (state 0, index 5) => +cs, file (state!=0, name>17) => -strnum
      for (long j = ip; j >= 0 && nt < MAXTEX; j--) {
        instate *q = (j == ip) ? p_curinput : &(*p_inputstack)[j];
        if (q->state == 0) { if (q->index == 5) s->tex[nt++] = (int)q->name; }
        else if (q->name > 17) s->tex[nt++] = -(int)q->name;
      }
      s->ntex = nt;
    }
  }
  *__errno_location() = save;
}

__attribute__((constructor(101))) static void prof_ctor(void) {
  t_ctor = now();
  const char *e = getenv("PROF_T0"); t0 = e ? strtoull(e, 0, 10) : t_ctor;
  pq = mmap(0, NEV * 2 * sizeof(long long), PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  evs = mmap(0, NEV * sizeof(ev_t), PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  samps = mmap(0, NSAMP * sizeof(samp_t), PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
  pathbuf = mmap(0, 1 << 24, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

static int phdr_cb(struct dl_phdr_info *i, size_t sz, void *d) { (void)sz; (void)d; exe_base = i->dlpi_addr; return 1; }
static long perf_open(u64 type, u64 config) {
  struct perf_event_attr a; memset(&a, 0, sizeof a); a.size = sizeof a; a.type = type; a.config = config; a.exclude_kernel = 1; a.exclude_hv = 1; a.disabled = 0;
  return syscall(SYS_perf_event_open, &a, 0, -1, -1, 0);
}
static void add_perf(const char *n, u64 type, u64 cfg) { if (npf >= NPERF) return; long fd = perf_open(type, cfg); if (fd >= 0) { pfd[npf] = fd; pname[npf] = n; npf++; } else { fprintf(stderr, "perf %s fail\n", n);} }
#define CACHE(id, op, res) ((id) | ((op) << 8) | ((res) << 16))
static void setup_perf(void) {
  const char *e = getenv("PROF_PERF"); if (!e) return;
  if (e[0]=='1') { add_perf("cycles", PERF_TYPE_HARDWARE, PERF_COUNT_HW_CPU_CYCLES); add_perf("instructions", PERF_TYPE_HARDWARE, PERF_COUNT_HW_INSTRUCTIONS); add_perf("branches", PERF_TYPE_HARDWARE, PERF_COUNT_HW_BRANCH_INSTRUCTIONS); add_perf("branch-misses", PERF_TYPE_HARDWARE, PERF_COUNT_HW_BRANCH_MISSES); }
  else if (e[0]=='2') {
    add_perf("cycles", PERF_TYPE_HARDWARE, PERF_COUNT_HW_CPU_CYCLES);
    add_perf("L1d-loads", PERF_TYPE_HW_CACHE, CACHE(PERF_COUNT_HW_CACHE_L1D, PERF_COUNT_HW_CACHE_OP_READ, PERF_COUNT_HW_CACHE_RESULT_ACCESS));
    add_perf("L1d-misses", PERF_TYPE_HW_CACHE, CACHE(PERF_COUNT_HW_CACHE_L1D, PERF_COUNT_HW_CACHE_OP_READ, PERF_COUNT_HW_CACHE_RESULT_MISS));
    add_perf("dTLB-misses", PERF_TYPE_HW_CACHE, CACHE(PERF_COUNT_HW_CACHE_DTLB, PERF_COUNT_HW_CACHE_OP_READ, PERF_COUNT_HW_CACHE_RESULT_MISS));
  } else if (e[0]=='3') {
    add_perf("cycles", PERF_TYPE_HARDWARE, PERF_COUNT_HW_CPU_CYCLES);
    add_perf("cache-refs", PERF_TYPE_HARDWARE, PERF_COUNT_HW_CACHE_REFERENCES);
    add_perf("cache-misses", PERF_TYPE_HARDWARE, PERF_COUNT_HW_CACHE_MISSES);
    add_perf("iTLB-misses", PERF_TYPE_HW_CACHE, CACHE(PERF_COUNT_HW_CACHE_ITLB, PERF_COUNT_HW_CACHE_OP_READ, PERF_COUNT_HW_CACHE_RESULT_MISS));
  }
  // hw breakpoints: PROF_BP="name=hexoff,name=hexoff"
  const char *b = getenv("PROF_BP");
  if (b) {
    static char bb[1024]; strncpy(bb, b, 1023); char *sv = 0;
    for (char *tok = strtok_r(bb, ",", &sv); tok && nbp < 4; tok = strtok_r(0, ",", &sv)) {
      char *eq = strchr(tok, '='); if (!eq) continue; *eq = 0;
      struct perf_event_attr a; memset(&a, 0, sizeof a); a.size = sizeof a; a.type = PERF_TYPE_BREAKPOINT; a.bp_type = HW_BREAKPOINT_X; a.bp_addr = exe_base + strtoull(eq + 1, 0, 16); a.bp_len = sizeof(long); a.exclude_kernel = 1; a.exclude_hv = 1; a.sample_period = 0;
      long fd = syscall(SYS_perf_event_open, &a, 0, -1, -1, 0);
      if (fd >= 0) { bpfd[nbp] = fd; bpname[nbp] = tok; nbp++; } else { perror("bp"); }
    }
  }
}
static void start_timer(void) {
  dl_iterate_phdr(phdr_cb, 0);
  const char *te = getenv("PROF_TEX");
  if (te) { unsigned long a, b, c, d, e2, f; if (sscanf(te, "%lx,%lx,%lx,%lx,%lx,%lx", &a, &b, &c, &d, &e2, &f) == 6) {
    p_curinput = (instate *)(exe_base + a); p_inputstack = (instate **)(exe_base + b); p_inputptr = (long *)(exe_base + c); p_hash = (twoh **)(exe_base + d); p_strpool = (unsigned char **)(exe_base + e2); p_strstart = (long **)(exe_base + f); } }
  setup_perf();
  const char *e = getenv("PROF_US"); long us = e ? atol(e) : 500; if (us <= 0) return;
  void *dummy[4]; backtrace(dummy, 4); // force libgcc load before any signal arrives
  struct sigaction sa; memset(&sa, 0, sizeof sa); sa.sa_sigaction = on_prof; sa.sa_flags = SA_SIGINFO | SA_RESTART; sigfillset(&sa.sa_mask);
  sigaction(SIGPROF, &sa, 0);
  struct sigevent sev; memset(&sev, 0, sizeof sev); sev.sigev_notify = SIGEV_SIGNAL; sev.sigev_signo = SIGPROF;
  if (timer_create(CLOCK_MONOTONIC, &sev, &tid) == 0) {
    tid_ok = 1; struct itimerspec it; it.it_interval.tv_sec = us / 1000000; it.it_interval.tv_nsec = (us % 1000000) * 1000; it.it_value = it.it_interval;
    timer_settime(tid, 0, &it, 0); active = 1;
  }
}

static void dump(void) {
  static int done; if (done) return; done = 1;
  u64 t_end = now();
  active = 0; if (tid_ok) timer_delete(tid);
  const char *out = getenv("PROF_OUT"); if (!out) return;
  int (*ropen)(const char *, int, ...) = dlsym(RTLD_NEXT, "open");
  ssize_t (*rwrite)(int, const void *, size_t) = dlsym(RTLD_NEXT, "write");
  ssize_t (*rread)(int, void *, size_t) = dlsym(RTLD_NEXT, "read");
  int fd = ropen(out, O_WRONLY | O_CREAT | O_TRUNC, 0644);
  if (fd < 0) return;
  struct rusage ru; getrusage(RUSAGE_SELF, &ru);
  char buf[4096];
  int l = snprintf(buf, sizeof buf, "H t0 %llu ctor %llu main %llu end %llu nev %d nsamp %d maxrss %ld minflt %ld majflt %ld ut %ld st %ld\n",
    (unsigned long long)t0, (unsigned long long)t_ctor, (unsigned long long)t_main, (unsigned long long)t_end, nev, nsamp, ru.ru_maxrss, ru.ru_minflt, ru.ru_majflt,
    ru.ru_utime.tv_sec * 1000000 + ru.ru_utime.tv_usec, ru.ru_stime.tv_sec * 1000000 + ru.ru_stime.tv_usec);
  rwrite(fd, buf, l);
  for (int i = 0; i < C__N; i++) { l = snprintf(buf, sizeof buf, "C %s %llu %llu %llu\n", cname[i], (unsigned long long)ccnt[i], (unsigned long long)ctot[i], (unsigned long long)cbytes[i]); rwrite(fd, buf, l); }
  { for (int i = 0; i < npf; i++) { long long v = 0; ssize_t rr = rread(pfd[i], &v, sizeof v); (void)rr; l = snprintf(buf, sizeof buf, "P %s %lld\n", pname[i], v); rwrite(fd, buf, l); }
    for (int i = 0; i < nbp; i++) { long long v = 0; ssize_t rr = rread(bpfd[i], &v, sizeof v); (void)rr; l = snprintf(buf, sizeof buf, "B %s %lld\n", bpname[i], v); rwrite(fd, buf, l); } }
  // /proc/self/maps
  int mf = ropen("/proc/self/maps", O_RDONLY); char mb[65536]; ssize_t r; rwrite(fd, "MAPS\n", 5);
  while ((r = rread(mf, mb, sizeof mb)) > 0) rwrite(fd, mb, r); close(mf); rwrite(fd, "ENDMAPS\n", 8);
  for (int i = 0; i < nev; i++) { l = snprintf(buf, sizeof buf, "E %llu %d %ld %ld %s\n", (unsigned long long)evs[i].t, evs[i].kind, evs[i].a, evs[i].b, evs[i].pathoff >= 0 ? pathbuf + evs[i].pathoff : "-"); rwrite(fd, buf, l); if (npf >= 2 && pq[i][0]) { l = snprintf(buf, sizeof buf, "Q %llu %lld %lld %d %s\n", (unsigned long long)evs[i].t, pq[i][0], pq[i][1], evs[i].kind, evs[i].pathoff >= 0 ? pathbuf + evs[i].pathoff : "-"); rwrite(fd, buf, l); } }
  // samples: text lines (compact hex)
  FILE *sf = 0; (void)sf;
  static char lb[32768];
  for (int i = 0; i < nsamp; i++) {
    samp_t *s = &samps[i]; int p = snprintf(lb, sizeof lb, "S %llu %llx", (unsigned long long)s->t, (unsigned long long)s->rip);
    for (int j = 0; j < s->n; j++) p += snprintf(lb + p, sizeof lb - p, " %llx", (unsigned long long)s->fr[j]);
    lb[p++] = '\n'; rwrite(fd, lb, p);
  }
  // resolve TeX names used by samples
  if (p_hash && *p_hash) {
    int maxn = 1 << 20; char *seen = mmap(0, maxn, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    unsigned char *sp = *p_strpool; long *ss = *p_strstart; twoh *h = *p_hash;
    for (int i = 0; i < nsamp; i++) for (int j = 0; j < samps[i].ntex; j++) {
      int v = samps[i].tex[j]; int a = v < 0 ? -v : v; if (a >= maxn / 2) continue; int key = v < 0 ? a + maxn / 2 : a; if (seen[key]) continue; seen[key] = 1;
      int p = snprintf(lb, sizeof lb, "T %d ", v);
      long st = -1;
      if (v < 0) st = a;
      else if (v < 257) { p += snprintf(lb + p, 64, "~%c", v - 1 < 127 && v - 1 >= 33 ? v - 1 : '?'); }
      else if (v < 513) { p += snprintf(lb + p, 64, "%c", v - 257 < 127 && v - 257 >= 33 ? v - 257 : '?'); }
      else if (v == 513) { p += snprintf(lb + p, 64, "nullcs"); }
      else st = h[v].rh;
      if (st >= 0) { long b0 = ss[st], b1 = ss[st + 1]; for (long k = b0; k < b1 && p < 400; k++) lb[p++] = sp[k] >= 33 && sp[k] < 127 ? sp[k] : '?'; }
      lb[p++] = '\n'; rwrite(fd, lb, p);
    }
    // sample stacks
    for (int i = 0; i < nsamp; i++) { samp_t *x = &samps[i]; int p = snprintf(lb, sizeof lb, "X %d", i); for (int j = 0; j < x->ntex; j++) p += snprintf(lb + p, sizeof lb - p, " %d", x->tex[j]); lb[p++] = '\n'; rwrite(fd, lb, p); }
  }
  t_dump_end = now();
  l = snprintf(buf, sizeof buf, "D %llu\n", (unsigned long long)t_dump_end); rwrite(fd, buf, l);
  close(fd);
}

// main wrapper
static int (*real_main)(int, char **, char **);
static int wrap_main(int ac, char **av, char **env) { t_main = now(); addev(1, "main", 0, 0); start_timer(); return real_main(ac, av, env); }
int __libc_start_main(int (*main)(int, char **, char **), int argc, char **argv, int (*init)(int, char **, char **), void (*fini)(void), void (*rtld_fini)(void), void *stack_end) {
  int (*rs)(int (*)(int, char **, char **), int, char **, int (*)(int, char **, char **), void (*)(void), void (*)(void), void *) = dlsym(RTLD_NEXT, "__libc_start_main");
  real_main = main; return rs(wrap_main, argc, argv, init, fini, rtld_fini, stack_end);
}
void exit(int code) { void (*re)(int) = dlsym(RTLD_NEXT, "exit"); addev(2, "exit", code, 0); dump(); re(code); __builtin_unreachable(); }

#define TIMED(c, expr) ({ u64 _a = now(); __auto_type _r = (expr); ctot[c] += now() - _a; ccnt[c]++; _r; })

FILE *fopen(const char *p, const char *m) { static FILE *(*f)(const char *, const char *); if (!f) f = dlsym(RTLD_NEXT, "fopen"); FILE *r = TIMED(C_FOPEN, f(p, m)); addev(10, p, m && m[0] == 'r' ? (r ? 1 : -1) : (r ? 2 : -2), 0); return r; }
FILE *fopen64(const char *p, const char *m) { static FILE *(*f)(const char *, const char *); if (!f) f = dlsym(RTLD_NEXT, "fopen64"); FILE *r = TIMED(C_FOPEN, f(p, m)); addev(10, p, m && m[0] == 'r' ? (r ? 1 : -1) : (r ? 2 : -2), 0); return r; }
int fclose(FILE *s) { static int (*f)(FILE *); if (!f) f = dlsym(RTLD_NEXT, "fclose"); addev(11, 0, fileno(s), 0); return f(s); }
int open(const char *p, int fl, ...) { static int (*f)(const char *, int, ...); if (!f) f = dlsym(RTLD_NEXT, "open"); va_list a; va_start(a, fl); int mode = va_arg(a, int); va_end(a); int r = TIMED(C_OPEN, f(p, fl, mode)); addev(12, p, r, fl); return r; }
int open64(const char *p, int fl, ...) { static int (*f)(const char *, int, ...); if (!f) f = dlsym(RTLD_NEXT, "open64"); va_list a; va_start(a, fl); int mode = va_arg(a, int); va_end(a); int r = TIMED(C_OPEN, f(p, fl, mode)); addev(12, p, r, fl); return r; }
ssize_t read(int fd, void *b, size_t n) { static ssize_t (*f)(int, void *, size_t); if (!f) f = dlsym(RTLD_NEXT, "read"); ssize_t r = TIMED(C_READ, f(fd, b, n)); if (r > 0) cbytes[C_READ] += r; return r; }
ssize_t write(int fd, const void *b, size_t n) { static ssize_t (*f)(int, const void *, size_t); if (!f) f = dlsym(RTLD_NEXT, "write"); ssize_t r = TIMED(C_WRITE, f(fd, b, n)); if (r > 0) cbytes[C_WRITE] += r; return r; }
size_t fread(void *b, size_t s, size_t n, FILE *fp) { static size_t (*f)(void *, size_t, size_t, FILE *); if (!f) f = dlsym(RTLD_NEXT, "fread"); size_t r = TIMED(C_FREAD, f(b, s, n, fp)); cbytes[C_FREAD] += r * s; return r; }
size_t fwrite(const void *b, size_t s, size_t n, FILE *fp) { static size_t (*f)(const void *, size_t, size_t, FILE *); if (!f) f = dlsym(RTLD_NEXT, "fwrite"); size_t r = TIMED(C_FWRITE, f(b, s, n, fp)); cbytes[C_FWRITE] += r * s; return r; }
int fflush(FILE *s) { static int (*f)(FILE *); if (!f) f = dlsym(RTLD_NEXT, "fflush"); return TIMED(C_FFLUSH, f(s)); }
int stat(const char *p, struct stat *st) { static int (*f)(const char *, struct stat *); if (!f) f = dlsym(RTLD_NEXT, "stat"); int r = TIMED(C_STAT, f(p, st)); addev(13, p, r, 0); return r; }
int lstat(const char *p, struct stat *st) { static int (*f)(const char *, struct stat *); if (!f) f = dlsym(RTLD_NEXT, "lstat"); int r = TIMED(C_STAT, f(p, st)); addev(13, p, r, 1); return r; }
int access(const char *p, int m) { static int (*f)(const char *, int); if (!f) f = dlsym(RTLD_NEXT, "access"); int r = TIMED(C_ACCESS, f(p, m)); addev(14, p, r, m); return r; }
DIR *opendir(const char *p) { static DIR *(*f)(const char *); if (!f) f = dlsym(RTLD_NEXT, "opendir"); DIR *r = TIMED(C_OPENDIR, f(p)); addev(15, p, r ? 0 : -1, 0); return r; }

// zlib (pdftex links libz dynamically)
typedef struct gzFile_s *gzf;
int inflate(void *strm, int flush) { static int (*f)(void *, int); if (!f) f = dlsym(RTLD_NEXT, "inflate"); return TIMED(C_INFLATE, f(strm, flush)); }
int deflate(void *strm, int flush) { static int (*f)(void *, int); if (!f) f = dlsym(RTLD_NEXT, "deflate"); return TIMED(C_DEFLATE, f(strm, flush)); }
gzf gzdopen(int fd, const char *m) { static gzf (*f)(int, const char *); if (!f) f = dlsym(RTLD_NEXT, "gzdopen"); addev(20, m, fd, 0); return f(fd, m); }
int gzread(gzf g, void *b, unsigned n) { static int (*f)(gzf, void *, unsigned); if (!f) f = dlsym(RTLD_NEXT, "gzread"); int r = TIMED(C_GZREAD, f(g, b, n)); if (r > 0) cbytes[C_GZREAD] += r; return r; }
int gzclose(gzf g) { static int (*f)(gzf); if (!f) f = dlsym(RTLD_NEXT, "gzclose"); int r = f(g); addev(21, "gzclose", r, 0); return r; }
