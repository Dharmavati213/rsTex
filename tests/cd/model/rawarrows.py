import sys,re,os
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
import run as R
tex=sys.argv[1]
pdf,log,rc=R.compile(tex)
st=''.join(R.contents(pdf))
src=open(tex).read()
styles=re.findall(r'\\arrow\[([^\]]*)\]',src)
# pictures
parts=re.split(r'\nq \n0 G \n0 g \n0\.3985 w \n',st)
for i,p in enumerate(parts[1:]):
    # drop matrix: after the cell boxes there is 'Q \nQ \n' then path stuff
    idx=p.find('\nQ \nQ \nq \n')
    m=re.search(r'1 0 0 1 [-\d.]+ [-\d.]+ cm\nQ \nQ \n(.*?)Q \nn \nQ',p,re.S)
    body=m.group(1) if m else p
    body=re.sub(r' \n',' ; ',body)
    body=body.replace('\n',' ; ')
    print(f'[{i+1}] {styles[i] if i<len(styles) else "?"}')
    print('   ',body)
