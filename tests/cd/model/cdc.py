#!/usr/bin/env python3
"""Compile a .tex with pdflatex into the scratch output dir and dump page content streams.
usage: cdc.py file.tex [--raw] [--page N]
"""
import sys,os,subprocess,re,zlib
OUT='/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/tikzcd/out'
os.makedirs(OUT,exist_ok=True)
def compile(tex,passes=1):
    base=os.path.splitext(os.path.basename(tex))[0]
    d=os.path.dirname(os.path.abspath(tex))
    r=None
    for _ in range(passes):
        r=subprocess.run(['pdflatex','-interaction=nonstopmode','-halt-on-error','-output-directory',OUT,os.path.basename(tex)],cwd=d,capture_output=True,text=True,errors='replace')
    if r.returncode!=0:
        print(r.stdout[-3000:]); sys.exit(1)
    return os.path.join(OUT,base+'.pdf'),r.stdout
def contents(pdf):
    """return list of page content streams, in page order, using qpdf to normalise"""
    q=pdf+'.q.pdf'
    subprocess.run(['qpdf','--qdf','--object-streams=disable',pdf,q],check=False,capture_output=True)
    b=open(q,'rb').read()
    # find pages
    out=[]
    for m in re.finditer(rb'/Type /Page\b(?!s)',b):
        # locate object
        start=b.rfind(b' obj',0,m.start())
        end=b.find(b'endobj',m.start())
        ob=b[start:end]
        cm=re.search(rb'/Contents (\d+) 0 R',ob)
        n=int(cm.group(1))
        sm=re.search(rb'(?:^|\n)%d 0 obj.*?stream\n(.*?)\nendstream'%n,b,re.S)
        out.append(sm.group(1).decode('latin1'))
    return out
if __name__=='__main__':
    tex=sys.argv[1]
    pdf,log=compile(tex)
    pages=contents(pdf)
    for i,c in enumerate(pages):
        print(f'%%%% PAGE {i+1}')
        print(c)
