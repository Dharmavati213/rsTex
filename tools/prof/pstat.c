#define _GNU_SOURCE
#include <linux/perf_event.h>
#include <linux/hw_breakpoint.h>
#include <sys/syscall.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <sys/personality.h>
#include <unistd.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
static int po(struct perf_event_attr*a,pid_t pid){return syscall(SYS_perf_event_open,a,pid,-1,-1,0);}
struct ev{const char*n;int type;uint64_t cfg;};
#define HWC(id,op,res) ((id)|((op)<<8)|((res)<<16))
int main(int ac,char**av){
  // usage: pstat [-b name:offset]... -- cmd
  struct ev evs[]={{"cycles",PERF_TYPE_HARDWARE,PERF_COUNT_HW_CPU_CYCLES},{"instructions",PERF_TYPE_HARDWARE,PERF_COUNT_HW_INSTRUCTIONS},
   {"branches",PERF_TYPE_HARDWARE,PERF_COUNT_HW_BRANCH_INSTRUCTIONS},{"branch-misses",PERF_TYPE_HARDWARE,PERF_COUNT_HW_BRANCH_MISSES},
   {"cache-refs",PERF_TYPE_HARDWARE,PERF_COUNT_HW_CACHE_REFERENCES},{"cache-misses",PERF_TYPE_HARDWARE,PERF_COUNT_HW_CACHE_MISSES},
   {"L1d-read-miss",PERF_TYPE_HW_CACHE,HWC(PERF_COUNT_HW_CACHE_L1D,PERF_COUNT_HW_CACHE_OP_READ,PERF_COUNT_HW_CACHE_RESULT_MISS)},
   {"L1d-read-acc",PERF_TYPE_HW_CACHE,HWC(PERF_COUNT_HW_CACHE_L1D,PERF_COUNT_HW_CACHE_OP_READ,PERF_COUNT_HW_CACHE_RESULT_ACCESS)},
   {"dTLB-miss",PERF_TYPE_HW_CACHE,HWC(PERF_COUNT_HW_CACHE_DTLB,PERF_COUNT_HW_CACHE_OP_READ,PERF_COUNT_HW_CACHE_RESULT_MISS)},
   {"page-faults",PERF_TYPE_SOFTWARE,PERF_COUNT_SW_PAGE_FAULTS},{"task-clock-ns",PERF_TYPE_SOFTWARE,PERF_COUNT_SW_TASK_CLOCK}};
  int ne=sizeof evs/sizeof evs[0];
  int i=1; const char*bn[32]; uint64_t bo[32]; int nb=0;
  while(i<ac&&strcmp(av[i],"--")){ if(!strcmp(av[i],"-b")){char*s=strdup(av[i+1]);char*c=strchr(s,':');*c=0;bn[nb]=s;bo[nb]=strtoull(c+1,0,16);nb++;i+=2;} else i++; }
  i++;
  int p1[2],p2[2]; pipe(p1);
  pid_t pid=fork();
  if(!pid){ personality(ADDR_NO_RANDOMIZE); char c; read(p1[0],&c,1); int d=open("/dev/null",1); dup2(d,1); dup2(d,2); execvp(av[i],av+i); _exit(127);}
  int fds[64]; int nf=0;
  if(!nb) for(int k=0;k<ne;k++){ struct perf_event_attr a; memset(&a,0,sizeof a); a.size=sizeof a; a.type=evs[k].type; a.config=evs[k].cfg; a.disabled=1; a.enable_on_exec=1; a.exclude_kernel=1; a.exclude_hv=1; a.inherit=1;
     int fd=po(&a,pid); if(fd<0) perror(evs[k].n); fds[k]=fd; }
  for(int k=0;k<nb;k++){ struct perf_event_attr a; memset(&a,0,sizeof a); a.size=sizeof a; a.type=PERF_TYPE_BREAKPOINT; a.bp_type=HW_BREAKPOINT_X; a.bp_addr=0x555555554000ULL+bo[k]; a.bp_len=sizeof(long); a.disabled=1; a.enable_on_exec=1; a.exclude_kernel=1;a.exclude_hv=1;
     int fd=po(&a,pid); if(fd<0) perror(bn[k]); fds[k]=fd; }
  write(p1[1],"x",1);
  int st; waitpid(pid,&st,0);
  if(!nb) for(int k=0;k<ne;k++){ uint64_t v=0; if(fds[k]>=0) read(fds[k],&v,8); printf("%-16s %14llu\n",evs[k].n,(unsigned long long)v);}
  for(int k=0;k<nb;k++){ uint64_t v=0; if(fds[k]>=0) read(fds[k],&v,8); printf("%-16s %14llu\n",bn[k],(unsigned long long)v);}
  return 0;
}
