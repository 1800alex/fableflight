# Usage: python3 assets/aircraft/c172_liveries.py assets/aircraft/c172.png assets/aircraft/c172_liveries
# (c172.png itself is the navy & red picture; rerun after re-shooting it or
# changing c172::LIVERIES, whose colours are repeated in L below.)
# Makes one aircraft-picker picture per Cessna scheme from assets/aircraft/c172.png
# (painted in the original navy & red). Painted pixels are found by colour
# (navy: blue well above red, dropping one-pixel panel seams; red: red above
# green and blue). Each is moved in OKLab by the difference between the
# picture's typical navy (or red) and the new paint seen in the same light,
# which keeps its shading and reflections.
import sys, statistics
from PIL import Image, ImageFilter
src, out = sys.argv[1], sys.argv[2]
def s2l(c): c/=255; return c/12.92 if c<=0.04045 else ((c+0.055)/1.055)**2.4
def l2s(c): c=max(0,min(1,c)); return round(255*(c*12.92 if c<=0.0031308 else 1.055*c**(1/2.4)-0.055))
def ss(a,b,x): t=max(0,min(1,(x-a)/(b-a))); return t*t*(3-2*t)
def lab(c):
    r,g,b=c
    l=(0.4122214708*r+0.5363325363*g+0.0514459929*b)**(1/3); m=(0.2119034982*r+0.6806995451*g+0.1073969566*b)**(1/3); s=(0.0883024619*r+0.2817188376*g+0.6299787005*b)**(1/3)
    return (0.2104542553*l+0.7936177850*m-0.0040720468*s, 1.9779984951*l-2.4285922050*m+0.4505937099*s, 0.0259040371*l+0.7827717662*m-0.8086757660*s)
def rgb(L):
    l_=L[0]+0.3963377774*L[1]+0.2158037573*L[2]; m_=L[0]-0.1055613458*L[1]-0.0638541728*L[2]; s_=L[0]-0.0894841775*L[1]-1.2914855480*L[2]
    l,m,s=l_**3,m_**3,s_**3
    return (4.0767416621*l-3.3077115913*m+0.2309699292*s, -1.2684380046*l+2.6097574011*m-0.3413193965*s, -0.0041960863*l-0.7034186147*m+1.7076127010*s)
Y=lambda c: 0.2126*c[0]+0.7152*c[1]+0.0722*c[2]
W=(0.80,0.80,0.78); N=(0.010,0.024,0.075); R=(0.34,0.018,0.012)
L=[("none",None),("navy",(N,R)),("royal",((0.025,0.12,0.48),(0.20,0.45,0.80))),("green",((0.035,0.22,0.12),(0.86,0.68,0.12))),("burgundy",((0.30,0.025,0.075),(0.88,0.72,0.48)))]
im=Image.open(src).convert("RGBA"); px=im.load(); w,h=im.size
mn=Image.new("L",(w,h)); mr=Image.new("L",(w,h)); pn=mn.load(); pr=mr.load()
for y in range(h):
    for x in range(w):
        r,g,b,al=px[x,y]
        if al==0: continue
        mx=max(r,g,b,1)
        pn[x,y]=round(255*ss(0.15,0.22,(b-r)/mx)); pr[x,y]=round(255*ss(0.025,0.07,(r-max(g,b))/mx))
# (one-pixel seams out; then a pixel more all round, for the edges' blend)
mn=mn.filter(ImageFilter.MinFilter(3)).filter(ImageFilter.MaxFilter(3))
# only where the original is bluish/reddish at all, so the growth stays on paint
raw=im.convert("RGB")
def grow(m, test):
    g=m.filter(ImageFilter.MaxFilter(3)); gp=g.load(); mp=m.load(); rp=raw.load()
    for y in range(h):
        for x in range(w):
            if gp[x,y]>mp[x,y] and not test(*rp[x,y]): gp[x,y]=mp[x,y]
    return g
mn=grow(mn, lambda r,g,b: b-r>4); mr=grow(mr, lambda r,g,b: r-max(g,b)>3)
pn=mn.load(); pr=mr.load()
def ref(mask,thr=250):
    ls=[lab([s2l(v) for v in px[x,y][:3]]) for y in range(h) for x in range(w) if mask[x,y]>=thr and px[x,y][3]==255]
    return tuple(statistics.median(c[i] for c in ls) for i in range(3)), len(ls)
import math
# The white paint around each painted pixel (in the same light): OKLab of the
# unpainted, light, opaque pixels nearby, averaged.
R_=14
acc=[[0.0]*4 for _ in range(w*h)]
lw=[[None]*w for _ in range(h)]
src_l=[[None]*w for _ in range(h)]
for y in range(h):
    for x in range(w):
        r,g,b,al=px[x,y]
        if al==255 and pn[x,y]==0 and pr[x,y]==0:
            c=lab([s2l(r),s2l(g),s2l(b)])
            if c[0]>0.8: src_l[y][x]=c
# summed-area tables
def sat(f):
    T=[[0.0]*(w+1) for _ in range(h+1)]
    for y in range(h):
        row=0.0
        for x in range(w):
            row+=f(x,y); T[y+1][x+1]=T[y][x+1]+row
    return T
Ts=[sat(lambda x,y,i=i: src_l[y][x][i] if src_l[y][x] else 0.0) for i in range(3)]
Tn=sat(lambda x,y: 1.0 if src_l[y][x] else 0.0)
def box(T,x,y,r):
    x0,y0,x1,y1=max(0,x-r),max(0,y-r),min(w,x+r+1),min(h,y+r+1)
    return T[y1][x1]-T[y0][x1]-T[y1][x0]+T[y0][x0]
def local_white(x,y):
    r=R_
    while r<200:
        n=box(Tn,x,y,r)
        if n>=20: return tuple(box(Ts[i],x,y,r)/n for i in range(3))
        r*=2
    return lab([s2l(v) for v in (228,230,232)])
def C(c): return math.hypot(c[1],c[2])
LW,LN,LR=lab(W),lab(N),lab(R)
for name,sch in L:
    o=im.copy(); op=o.load()
    for y in range(h):
        for x in range(w):
            a,b_=pn[x,y]/255, pr[x,y]/255
            if a<=0 and b_<=0: continue
            r,g,b,al=px[x,y]; lin=[s2l(r),s2l(g),s2l(b)]; P=lab(lin)
            lwc=local_white(x,y)
            if a>=b_: m,base,which=a,LN,0
            else: m,base,which=b_,LR,1
            T=LW if sch is None else lab(sch[which])
            ql=min(lwc[0], P[0]*(T[0]/base[0]))
            if sch is None: qa,qb=lwc[1],lwc[2]
            else:
                c=C(P)*C(T)/max(C(base),1e-6); h_=math.atan2(T[2],T[1])
                qa,qb=c*math.cos(h_),c*math.sin(h_)
            new=rgb((ql,qa,qb))
            op[x,y]=tuple(l2s(lin[i]*(1-m)+new[i]*m) for i in range(3))+(al,)
    if sch is None:
        # No scheme is no paint: the white airframe (light, near-grey pixels)
        # turned to bare aluminium (shader: 0.62, 0.63, 0.65, metallic), its
        # lightness scaled down and given a cool cast, so it
        # reads as metal rather than white paint.
        AL=lab((0.62,0.63,0.65)); k=AL[0]/LW[0]
        for y in range(h):
            for x in range(w):
                r,g,b,al=op[x,y]
                if al==0: continue
                lin=[s2l(r),s2l(g),s2l(b)]; P=lab(lin)
                m=ss(0.55,0.75,P[0])*(1-ss(0.03,0.06,C(P)))
                if m<=0: continue
                ql=max(0,min(1,P[0]*k*0.94-0.04))
                new=rgb((ql,P[1]*0.5+AL[1],P[2]*0.5+AL[2]-0.006))
                op[x,y]=tuple(l2s(lin[i]*(1-m)+new[i]*m) for i in range(3))+(al,)
    if name != "navy": o.save(f"{out}/c172_{name}.png", optimize=True)
