#!/usr/bin/env python3
"""Tiny PDF content-stream interpreter: list draw/fill/text operations with absolute coordinates."""
import re
TOK=re.compile(r'\[(?:[^\]\\]|\\.|\((?:[^)\\]|\\.)*\))*\]|\((?:[^)\\]|\\.)*\)|/[^\s/\[\]()<>]+|<[0-9A-Fa-f]*>|[^\s\[\]()<>/]+')
def mul(a,b):
    return (a[0]*b[0]+a[1]*b[2], a[0]*b[1]+a[1]*b[3], a[2]*b[0]+a[3]*b[2], a[2]*b[1]+a[3]*b[3], a[4]*b[0]+a[5]*b[2]+b[4], a[4]*b[1]+a[5]*b[3]+b[5])
def app(m,x,y): return (m[0]*x+m[2]*y+m[4], m[1]*x+m[3]*y+m[5])
def parse(stream):
    gs=dict(ctm=(1,0,0,1,0,0),w=1.0,dash=([],0.0),cap=0,join=0,RG=(0,0,0),rg=(0,0,0),ml=10)
    stack=[];path=[];out=[];stk=[];tm=None;font=None
    last=[]
    for t in TOK.findall(stream):
        if re.fullmatch(r'[-+]?\d*\.?\d+',t): stk.append(float(t)); continue
        if t[0] in '[(/<': stk.append(t); continue
        op=t
        if op=='q': stack.append(dict(gs))
        elif op=='Q': gs=stack.pop()
        elif op=='cm': gs['ctm']=mul(tuple(stk[-6:]),gs['ctm'])
        elif op=='w':
            gs['w']=stk[-1]
            if abs(stk[-1]-0.3985)<1e-6 and len(last)>=3 and last[-3]=='q' and last[-2] in ('G','RG','K') and last[-1] in ('g','rg','k'):
                out.append(dict(kind='pic',ctm=gs['ctm']))
        elif op=='d':
            gs['dash']=([float(x) for x in re.findall(r'[-\d.]+',stk[-2])],stk[-1])
            gs['texrule']=(stk[-2]=='[]')
        elif op=='J': gs['cap']=int(stk[-1])
        elif op=='j': gs['join']=int(stk[-1])
        elif op=='M': gs['ml']=stk[-1]
        elif op=='RG': gs['RG']=tuple(stk[-3:])
        elif op=='rg': gs['rg']=tuple(stk[-3:])
        elif op=='G': gs['RG']=(stk[-1],)
        elif op=='g': gs['rg']=(stk[-1],)
        elif op=='K': gs['RG']=tuple(stk[-4:])
        elif op=='k': gs['rg']=tuple(stk[-4:])
        elif op=='m': path.append(('m',)+app(gs['ctm'],*stk[-2:]))
        elif op=='l': path.append(('l',)+app(gs['ctm'],*stk[-2:]))
        elif op=='c':
            a=stk[-6:]; path.append(('c',)+app(gs['ctm'],a[0],a[1])+app(gs['ctm'],a[2],a[3])+app(gs['ctm'],a[4],a[5]))
        elif op=='h': path.append(('h',))
        elif op=='re':
            a=stk[-4:]; path.append(('re',)+app(gs['ctm'],a[0],a[1])+(a[2],a[3]))
        elif op in('S','s','f','F','f*','B','B*','b','n'):
            if op!='n' and path:
                out.append(dict(kind='rule' if gs.get('texrule') else 'path',op=op,path=path,w=gs['w'],dash=gs['dash'],cap=gs['cap'],join=gs['join'],RG=gs['RG'],rg=gs['rg'],ctm=gs['ctm']))
            path=[]
        elif op=='BT': tm=(1,0,0,1,0,0)
        elif op=='ET': tm=None
        elif op=='Tf': font=(stk[-2],stk[-1])
        elif op=='Td':
            tm=mul((1,0,0,1,stk[-2],stk[-1]),tm)
        elif op in('TJ','Tj'):
            p=app(gs['ctm'],tm[4],tm[5]); out.append(dict(kind='text',at=p,font=font,s=stk[-1],rg=gs['rg']))
        last.append(op); last=last[-4:]
        stk=[]
    return out
def fmt(o,origin=(0,0),scale=1.0):
    ox,oy=origin
    if o['kind']=='rule': return "TEXRULE w=%.3f %s"%(o['w'],o['path'])
    if o['kind']=='pic': return f"PIC origin=({o['ctm'][4]:.3f},{o['ctm'][5]:.3f})"
    if o['kind']=='text':
        return f"TEXT at=({o['at'][0]-ox:.3f},{o['at'][1]-oy:.3f}) font={o['font']} {o['s'][:40]} rg={o['rg']}"
    parts=[]
    for seg in o['path']:
        if seg[0]=='h': parts.append('h'); continue
        if seg[0]=='re':
            parts.append(f"re {seg[1]-ox:.5f} {seg[2]-oy:.5f} {seg[3]:.5f} {seg[4]:.5f}"); continue
        co=seg[1:]; s=[]
        for i in range(0,len(co),2): s.append(f"{co[i]-ox:.5f} {co[i+1]-oy:.5f}")
        parts.append(seg[0]+' '+' '.join(s))
    return f"{o['op']} w={o['w']:.5f} d={o['dash'][0]}@{o['dash'][1]} J{o['cap']} j{o['join']} RG={o['RG']} rg={o['rg']} :: "+' ; '.join(parts)
