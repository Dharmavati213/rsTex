"""A document that carries a large passthrough image: the output cache must read no more of the old PDF than the index names.  The bar is 1.02 of no cache in CPU and wall.  `none2` is a second no cache, for the noise."""
import os, shutil, statistics, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import crit
E = crit.engine('rstex-outcache')
root = crit.fresh('bigpdf'); src=root+'/s'; out=root+'/o'; cache=root+'/c'
os.makedirs(src); os.makedirs(out); os.makedirs(cache)
j = open('/usr/share/texmf-dist/tex/latex/mwe/example-image.jpg','rb').read()
MB=int(sys.argv[1])
open(src+'/big.jpg','wb').write(j + os.urandom(1<<20)*MB)   # pdfTeX copies a JPEG's whole file into the stream
open(src+'/d.tex','w').write(r'\documentclass{article}\usepackage{graphicx}\begin{document}Hello world.\par\includegraphics[width=3cm]{big.jpg}\end{document}'+'\n')
env=crit.base_env(cache); env['RSTEX_CHECKPOINT_POLICY']='always'
argv=crit.latex_argv(E,'d.tex',out)
for _ in range(3): crit.run(argv,src,env)
print('pdf MB', os.path.getsize(out+'/d.pdf')/1e6)
tr=subprocess.run(argv,cwd=src,env=dict(env,RSTEX_OUTCACHE_TRACE='1'),stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE).stderr.decode('latin-1')
print([l for l in tr.splitlines() if l.startswith('rstex: outcache:')])
R={'hits':[],'none':[],'none2':[]}
sides=[('none',{'RSTEX_NO_OUTCACHE':'1'}),('none2',{'RSTEX_NO_OUTCACHE':'1'}),('hits',{})]
for r in range(int(os.environ.get('ROUNDS','41'))):
    for k in range(3):
        name,extra=sides[(r+k)%3]
        R[name].append(crit.counted(argv,src,dict(env,**extra)))
for k,L in R.items():
    md=lambda x: statistics.median(a[x] for a in L)
    ra=lambda x: statistics.median(a[x]/b[x] for a,b in zip(L,R['none']))
    print(f'{k:<6} instr {md("instr"):>8.2f} ({ra("instr"):.3f}) cycles {md("cyc"):>8.1f} ({ra("cyc"):.3f}) cpu min {min(a["cpu"] for a in L):>7.1f} med {md("cpu"):>7.1f} ({ra("cpu"):.3f}) sys med {md("sys"):>7.1f} wall min {min(a["wall"] for a in L):>7.1f} med {md("wall"):>7.1f} ({ra("wall"):.3f}) rss MB {md("rss")/1024:>7.0f}')
shutil.rmtree(root)
