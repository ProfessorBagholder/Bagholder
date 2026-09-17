import json,urllib.request,sys
paths=sys.argv[1:]
def get(port,p):
    r=urllib.request.Request("http://127.0.0.1:%d%s"%(port,p),headers={"Host":"127.0.0.1:%d"%port})
    try: return json.loads(urllib.request.urlopen(r,timeout=120).read())
    except urllib.error.HTTPError as e: return {"_http":e.code,"body":json.loads(e.read() or b"{}")}
IGN={"generated","startedAt","fetchedAt","dataVersion","path","refreshedAt"}
def diff(a,b,path=""):
    out=[]
    if isinstance(a,dict) and isinstance(b,dict):
        for k in list(dict.fromkeys(list(a)+list(b))):
            if k in IGN: continue
            if k not in a: out.append(path+"/"+k+" only rust")
            elif k not in b: out.append(path+"/"+k+" only py")
            else: out+=diff(a[k],b[k],path+"/"+k)
        if not out and list(a)!=list(b) and not (set(a)&IGN): out.append(path+" key order")
    elif isinstance(a,list) and isinstance(b,list):
        if len(a)!=len(b): out.append("%s len py=%d rs=%d"%(path,len(a),len(b)))
        for i,(x,y) in enumerate(zip(a,b)): out+=diff(x,y,"%s[%d]"%(path,i))
    else:
        if a!=b and not (isinstance(a,(int,float)) and isinstance(b,(int,float)) and not isinstance(a,bool) and abs(a-b)<1e-9): out.append("%s py=%r rs=%r"%(path,str(a)[:80],str(b)[:80]))
    return out
for p in paths:
    d=diff(get(8812,p),get(8811,p))
    print(p, "OK" if not d else "%d diffs"%len(d)); [print("   ",x) for x in d[:8]]
