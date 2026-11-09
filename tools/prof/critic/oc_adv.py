"""The output cache against a run with RSTEX_NO_OUTCACHE=1, after changes the key might not see.  Public, generated.  RSTEX_CRITIC_ENGINE is the binary judged.  Its last line says which failed, and the exit status agrees."""
import os, shutil, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import crit
def png_alpha(path, w=8, h=8):
    import zlib, struct
    def ch(t, d):
        c = struct.pack('>I', len(d)) + t + d
        return c + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    raw = b''.join(b'\0' + b''.join(bytes((x*30%256, y*30%256, 128, 200)) for x in range(w)) for y in range(h))
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b''))
BN = 'rstex-outcache'
E = crit.engine(BN)
root = crit.fresh('ocadv')
src, out, cache = root + '/src', root + '/o', root + '/c'
os.makedirs(src); os.makedirs(out); os.makedirs(cache)
for n in ('example-image.png', 'example-image.jpg', 'example-image.pdf'):
    shutil.copy('/usr/share/texmf-dist/tex/latex/mwe/' + n, src)
png_alpha(src + '/alpha.png')
shutil.copy('/usr/share/texmf-dist/fonts/type1/public/amsfonts/cm/cmss10.pfb', src + '/mycmss.pfb')
ENC0 = open('/usr/share/texmf-dist/fonts/enc/dvips/base/8r.enc', 'rb').read() if os.path.exists('/usr/share/texmf-dist/fonts/enc/dvips/base/8r.enc') else None
print('8r present', bool(ENC0))
BASE = r'''%PRE
\documentclass{article}
\usepackage[T1]{fontenc}
\usepackage{graphicx}
\usepackage{microtype}
\pdfgentounicode=1
\input{glyphtounicode}
%SET
\begin{document}
%BODY
\section{Text}
Regular text with \textbf{bold}, \textit{italic}, \texttt{typewriter} and $x^2+\alpha$ math.
{\fontencoding{OT1}\fontfamily{cmss}\selectfont sans text abcdef}
{\fontencoding{OT1}\fontfamily{cmr}\selectfont Roman OT1 text}
\pdfximage width 3cm {example-image.png}\pdfrefximage\pdflastximage
\pdfximage width 2cm {alpha.png}\pdfrefximage\pdflastximage
\pdfximage width 2cm {example-image.jpg}\pdfrefximage\pdflastximage
\includegraphics[width=3cm]{example-image.pdf}
\end{document}
'''
def write(tex):
    open(src + '/d.tex', 'w').write(tex)
env = crit.base_env(cache); env['RSTEX_CHECKPOINT_POLICY'] = 'always'
argv = crit.latex_argv(E, 'd.tex', out)
results = []
def case(name, settings='', pre='', body='', world=None, unworld=None, base_settings='', base_pre='', base_body='', old_file=None):
    """Base run twice (index filled), then the changed world, with and without the cache."""
    tb = BASE.replace('%SET', base_settings).replace('%PRE', base_pre).replace('%BODY', base_body)
    write(tb)
    shutil.rmtree(out); os.makedirs(out)
    shutil.rmtree(cache + '/rstex/outcache', ignore_errors=True)
    for _ in range(3):
        crit.run(argv, src, env)
    time.sleep(2.1)  # a font's stamp must be 2 s old to be kept
    crit.run(argv, src, env)
    prev = crit.snapshot(out)
    if world: world()
    write(BASE.replace('%SET', settings or base_settings).replace('%PRE', pre or base_pre).replace('%BODY', body or base_body))
    rc0, so0, se0 = crit.run(argv, src, dict(env, RSTEX_NO_OUTCACHE='1'))
    ref = crit.snapshot(out)
    shutil.rmtree(out); os.makedirs(out)
    for k, v in prev.items(): open(os.path.join(out, k), 'wb').write(v)
    if old_file: old_file()
    rc1, so1, se1 = crit.run(argv, src, dict(env, RSTEX_OUTCACHE_TRACE='1'))
    got = crit.snapshot(out)
    tr = [l for l in se1.splitlines() if l.startswith('rstex: outcache:')]
    d = crit.differs(ref, got)
    ok = not d and rc0 == rc1 and so0 == so1
    print(f'{name:<34} rc {rc0}/{rc1} {"OK" if ok else "DIFF " + str(d)}  {tr[0][17:90] if tr else ""}', flush=True)
    results.append((name, ok))
    if unworld: unworld()
case('no change')
case('slant by map line', settings=r'\pdfmapline{=cmss10 CMSS10 "0.2 SlantFont" <cmss10.pfb}')
case('extend by map line', settings=r'\pdfmapline{=cmss10 CMSS10 "1.2 ExtendFont" <cmss10.pfb}')
case('other pfb by map line', settings=r'\pdfmapline{=cmss10 CMSS10 <cmssi10.pfb}')
case('project pfb by map line, same fd', settings=r'\pdfmapline{=cmss10 CMSS10 <mycmss.pfb}')
case('full embed <<', settings=r'\pdfmapline{=cmss10 CMSS10 <<cmss10.pfb}')
case('not embedded', settings=r'\pdfmapline{=cmss10 CMSS10}')
case('map line removed', settings=r'\pdfmapline{-cmss10}')
case('gentounicode off', settings=r'\pdfgentounicode=0')
case('glyph unicode changed', settings=r'\pdfglyphtounicode{a}{0062}')
case('compress 0', settings=r'\pdfcompresslevel=0')
case('compress 1', settings=r'\pdfcompresslevel=1')
case('compress 9', settings=r'\pdfcompresslevel=9')
case('objcompress 0', settings=r'\pdfobjcompresslevel=0')
case('minor 3', settings=r'\pdfminorversion=3')
case('minor 4', settings=r'\pdfminorversion=4')
case('gamma', settings=r'\pdfimageapplygamma=1 \pdfgamma=2200 \pdfimagegamma=1000')
case('hicolor off', settings=r'\pdfimagehicolor=0')
case('resolution', settings=r'\pdfimageresolution=300')
case('suppress ptexinfo', settings=r'\pdfsuppressptexinfo=-1')
case('inclusion copy fonts', settings=r'\pdfinclusioncopyfonts=1')
case('inclusion err level', settings=r'\pdfinclusionerrorlevel=-1')
case('omit charset', settings=r'\ifdefined\pdfomitcharset\pdfomitcharset=1\fi')
case('omit charset 2', settings=r'\ifdefined\pdfomitprocset\pdfomitprocset=1\fi')
case('tag via pdfpkresolution', settings=r'\pdfpkresolution=1200')
case('font order swapped', body=r'{\fontencoding{OT1}\fontfamily{cmr}\selectfont QXZ first}\texttt{tt first}\pdfincludechars\font{a}')
case('glyph added', body=r'\pdfincludechars\font{QQQ!}Q!')
case('one font more', body=r'{\Large\sffamily Added sans.}')
case('include chars via pdfincludechars of tt', body=r'{\ttfamily\pdfincludechars\font{@@@}}')
case('font expansion on', settings=r'\microtypesetup{expansion=true}\pdfadjustspacing=2')
case('letterspace', body=r'{\font\xx=cmr10 \letterspacefont\yy\xx 100 \yy spaced}')
ms = r'\pdfmapline{=ptmr8r NimbusRomNo9L-Regu " TeXBase1Encoding ReEncodeFont " <my8r.enc <utmr8a.pfb}\font\tt@x=ptmr8r \AtBeginDocument{\tt@x text in ptmr8r \textquoteright}'
if ENC0:
    def e0():
        open(src + '/my8r.enc', 'wb').write(ENC0)
    e0()
    def enc_change():
        d = ENC0
        # Same glyph set as far as the text goes: swap two names used by no glyph in the text.
        a, b = b'/quotesinglbase', b'/florin'
        d = d.replace(a, b'/XXXX').replace(b, a).replace(b'/XXXX', b)
        st = os.stat(src + '/my8r.enc')
        open(src + '/my8r.enc', 'wb').write(d)
        os.utime(src + '/my8r.enc', ns=(st.st_atime_ns, st.st_mtime_ns))
    case('project enc changed, same mtime', base_settings=ms, world=enc_change, unworld=e0)
    def enc_change2():
        d = ENC0.replace(b'/a ', b'/b ', 1)   # the glyph for a code in use moves
        open(src + '/my8r.enc', 'wb').write(d)
    case('project enc: a glyph in use moved', base_settings=ms, world=enc_change2, unworld=e0)
def pfb_world():
    d = open('/usr/share/texmf-dist/fonts/type1/public/amsfonts/cm/cmssi10.pfb', 'rb').read()
    st = os.stat(src + '/mycmss.pfb')
    open(src + '/mycmss.pfb', 'wb').write(d)
def pfb_back():
    shutil.copy('/usr/share/texmf-dist/fonts/type1/public/amsfonts/cm/cmss10.pfb', src + '/mycmss.pfb')
case('project pfb other bytes', settings=r'\pdfmapline{=cmss10 CMSS10 <mycmss.pfb}', base_settings=r'\pdfmapline{=cmss10 CMSS10 <mycmss.pfb}', world=pfb_world, unworld=pfb_back)
def png_world():
    d = open('/usr/share/texmf-dist/tex/latex/mwe/example-image-a.png', 'rb').read()
    open(src + '/example-image.png', 'wb').write(d)
def png_back():
    shutil.copy('/usr/share/texmf-dist/tex/latex/mwe/example-image.png', src)
case('png other', world=png_world, unworld=png_back)
def old_pdf_other():
    p = out + '/d.pdf'
    d = bytearray(open(p, 'rb').read()); d[len(d) // 2] ^= 1
    open(p, 'wb').write(bytes(d))
case('old pdf bit flipped', old_file=old_pdf_other)
def old_pdf_gone():
    os.unlink(out + '/d.pdf')
case('old pdf deleted', old_file=old_pdf_gone)
print('bad', [r[0] for r in results if not r[1]], 'of', len(results))
shutil.rmtree(root, ignore_errors=True)
sys.exit(1 if any(not r[1] for r in results) else 0)
