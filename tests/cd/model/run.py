#!/usr/bin/env python3
"""usage: run.py file.tex [--ops] [--log] [--raw]  ; compile with instrumentation and show ops per picture"""
import sys,os,subprocess,re
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
import pdfops
HERE=os.path.dirname(os.path.abspath(__file__))
OUT=os.path.join(os.path.dirname(HERE),'out')
os.makedirs(OUT,exist_ok=True)
def compile(tex,passes=1):
    base=os.path.splitext(os.path.basename(tex))[0]
    d=os.path.dirname(os.path.abspath(tex))
    env=dict(os.environ); env['max_print_line']='100000'; env['TEXINPUTS']=HERE+':'+d+':'+env.get('TEXINPUTS','')
    for _ in range(passes):
        r=subprocess.run(['pdflatex','-interaction=nonstopmode','-output-directory',OUT,'-jobname',base,'\\input{instr.tex}\\input{%s}'%os.path.basename(tex)],cwd=d,capture_output=True,env=env,errors='replace',text=True)
    log=open(os.path.join(OUT,base+'.log'),errors='replace').read()
    return os.path.join(OUT,base+'.pdf'),log,r.returncode
def contents(pdf):
    q=pdf+'.q.pdf'
    subprocess.run(['qpdf','--qdf','--object-streams=disable',pdf,q],check=False,capture_output=True)
    b=open(q,'rb').read()
    out=[]
    for m in re.finditer(rb'/Type /Page\b(?!s)',b):
        start=b.rfind(b' obj',0,m.start())
        end=b.find(b'endobj',m.start())
        ob=b[start:end]
        cm=re.search(rb'/Contents (\d+) 0 R',ob)
        n=int(cm.group(1))
        sm=re.search(rb'(?:^|\n)%d 0 obj.*?stream\n(.*?)\nendstream'%n,b,re.S)
        out.append(sm.group(1).decode('latin1'))
    return out
if __name__=='__main__':
    args=sys.argv[1:]
    tex=args[0]
    pdf,log,rc=compile(tex)
    errs=re.findall(r'^!.*(?:\n.*){0,3}',log,re.M)
    if errs:
        print('ERRORS:'); 
        for e in errs[:12]: print(e)
    if '--log' in args:
        for ln in log.split('\n'):
            if ln.startswith('CD'): print(ln)
    if '--raw' in args:
        for i,c in enumerate(contents(pdf)):
            print(f'%%%% PAGE {i+1}'); print(c)
    if '--ops' in args:
        for i,c in enumerate(contents(pdf)):
            print(f'%%%% PAGE {i+1}')
            ops=pdfops.parse(c)
            org=(0,0)
            for o in ops:
                if o["kind"]=="pic": org=(o["ctm"][4],o["ctm"][5])
                print(pdfops.fmt(o,org))
