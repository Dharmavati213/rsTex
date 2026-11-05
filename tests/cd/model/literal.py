import sys,re,os
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
import run as R
T=os.path.dirname(os.path.dirname(os.path.abspath(__file__)))+os.sep
out=[]
def proc(fn,only=None):
    tex=T+fn
    pdf,log,rc=R.compile(tex)
    st=''.join(R.contents(pdf))
    src=open(tex).read()
    src=re.sub(r'(?<!\\)%.*','',src)
    envs=re.findall(r'\\begin\{tikzcd\}(?:\[[^\]]*\])?(.*?)\\end\{tikzcd\}',src,re.S)
    heads=re.findall(r'\\begin\{tikzcd\}(\[[^\]]*\])?',src)
    parts=re.split(r'\nq \n0 G \n0 g \n0\.3985 w \n',st)[1:]
    for i,p in enumerate(parts):
        if i>=len(envs): break
        opts=re.findall(r'\\arrow\[([^\]]*)\]',envs[i])
        if len(opts)!=1: continue
        name=opts[0]
        # strip cells: everything up to the last 'Q \nQ \n' before the arrow's first w/q
        pp=re.split(r'1 0 0 1 [-\d.]+ [-\d.]+ cm\nQ \nQ \n',p)
        tail=pp[-1]
        e=tail.rfind('Q \nn \nQ')
        body=tail[:e]
        body=re.sub(r' *\n',' ',body).strip()
        # drop outer q .. Q
        if body.startswith('q ') and body.endswith('Q'):
            body=body[2:-1].strip()
        out.append((name,heads[i],body))
for f in ['03-heads-plain.tex','04-heads-hooks.tex','05-heads-squiggly-dashed.tex','06-end-modifiers.tex']:
    proc(f)
seen=set()
for n,h,b in out:
    key=(n,h)
    if key in seen: continue
    seen.add(key)
    n2=n.replace('r,','',1) if n.startswith('r,') else n
    print(f"{(h or '')+' ' if h else ''}{n2}:\n  {b}\n")
