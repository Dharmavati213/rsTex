#define _GNU_SOURCE
#include <signal.h>
#include <sys/time.h>
#include <ucontext.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#define MAXS 2000000
static unsigned long *buf; static volatile long n;
static void h(int s, siginfo_t*i, void*c){ ucontext_t*u=c; if(n<MAXS) buf[n++]=u->uc_mcontext.gregs[REG_RIP]; }
__attribute__((constructor)) static void init(void){
  const char *o=getenv("SHIM_OUT"); if(!o) return;
  buf=calloc(MAXS,8);
  struct sigaction sa; memset(&sa,0,sizeof sa); sa.sa_sigaction=h; sa.sa_flags=SA_SIGINFO|SA_RESTART; sigaction(SIGPROF,&sa,0);
  struct itimerval t={{0,200},{0,200}}; setitimer(ITIMER_PROF,&t,0);
}
__attribute__((destructor)) static void fini(void){
  const char *o=getenv("SHIM_OUT"); if(!o) return;
  struct itimerval t={{0,0},{0,0}}; setitimer(ITIMER_PROF,&t,0);
  char nm[512]; snprintf(nm,512,"%s.%d",o,getpid());
  FILE*f=fopen(nm,"w"); FILE*m=fopen("/proc/self/maps","r"); char l[1024];
  while(fgets(l,1024,m)) fprintf(f,"M %s",l); fclose(m);
  for(long i=0;i<n;i++) fprintf(f,"S %lx\n",buf[i]); fclose(f);
}
