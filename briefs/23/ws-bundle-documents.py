import re, glob, os, json, sys

def balanced(t, i):
    # t[i] == '{'; return end index of matching '}' honouring strings
    depth=0; j=i; n=len(t); instr=None
    while j<n:
        c=t[j]
        if instr:
            if c=='\\': j+=2; continue
            if c==instr: instr=None
        else:
            if c in '"\'`': instr=c
            elif c=='{': depth+=1
            elif c=='}':
                depth-=1
                if depth==0: return j
        j+=1
    return -1

def to_json(s):
    s=re.sub(r'([{,])\s*([A-Za-z_$][A-Za-z0-9_$]*)\s*:', r'\1"\2":', s)
    s=s.replace('!0','true').replace('!1','false')
    return json.loads(s)

def p_type(t):
    k=t['kind']
    if k=='NamedType': return t['name']['value']
    if k=='ListType': return '['+p_type(t['type'])+']'
    if k=='NonNullType': return p_type(t['type'])+'!'
    return '?'

def p_val(v):
    k=v['kind']
    if k=='Variable': return '$'+v['name']['value']
    if k in ('IntValue','FloatValue','EnumValue'): return v['value']
    if k=='StringValue': return json.dumps(v['value'])
    if k=='BooleanValue': return 'true' if v['value'] else 'false'
    if k=='NullValue': return 'null'
    if k=='ListValue': return '['+', '.join(p_val(x) for x in v['values'])+']'
    if k=='ObjectValue': return '{'+', '.join(f["name"]["value"]+': '+p_val(f['value']) for f in v['fields'])+'}'
    return '?'

def p_args(args):
    if not args: return ''
    return '('+', '.join(a['name']['value']+': '+p_val(a['value']) for a in args)+')'

def p_dirs(ds):
    return ''.join(' @'+d['name']['value']+p_args(d.get('arguments')) for d in (ds or []))

def p_sel(ss, ind):
    out=[]
    for s in ss['selections']:
        k=s['kind']; pad='  '*ind
        if k=='Field':
            name=s['name']['value']
            if s.get('alias'): name=s['alias']['value']+': '+name
            line=pad+name+p_args(s.get('arguments'))+p_dirs(s.get('directives'))
            if s.get('selectionSet'):
                out.append(line+' {'); out.extend(p_sel(s['selectionSet'], ind+1)); out.append(pad+'}')
            else: out.append(line)
        elif k=='FragmentSpread':
            out.append(pad+'...'+s['name']['value']+p_dirs(s.get('directives')))
        elif k=='InlineFragment':
            tc=s.get('typeCondition'); head=pad+'...'+(' on '+tc['name']['value'] if tc else '')+p_dirs(s.get('directives'))
            out.append(head+' {'); out.extend(p_sel(s['selectionSet'], ind+1)); out.append(pad+'}')
    return out

def p_def(d):
    k=d['kind']
    if k=='OperationDefinition':
        vs=d.get('variableDefinitions') or []
        head=d['operation']+(' '+d['name']['value'] if d.get('name') else '')
        if vs: head+='('+', '.join('$'+v['variable']['name']['value']+': '+p_type(v['type'])+(' = '+p_val(v['defaultValue']) if v.get('defaultValue') else '') for v in vs)+')'
        return '\n'.join([head+p_dirs(d.get('directives'))+' {']+p_sel(d['selectionSet'],1)+['}'])
    if k=='FragmentDefinition':
        head='fragment '+d['name']['value']+' on '+d['typeCondition']['name']['value']
        return '\n'.join([head+p_dirs(d.get('directives'))+' {']+p_sel(d['selectionSet'],1)+['}'])
    return '# '+k

def spreads(d):
    out=set()
    def walk(ss):
        for s in ss.get('selections',[]):
            if s['kind']=='FragmentSpread': out.add(s['name']['value'])
            if s.get('selectionSet'): walk(s['selectionSet'])
    walk(d['selectionSet']); return out

docs={}; frags={}; bad=0
for f in sorted(glob.glob('chunks/*.js'))+['app.js']:
    t=open(f,encoding='utf-8',errors='replace').read()
    for m in re.finditer(r'\{kind:"Document"', t):
        e=balanced(t, m.start())
        if e<0: bad+=1; continue
        raw=t[m.start():e+1]
        try: d=to_json(raw)
        except Exception as ex:
            bad+=1; continue
        for df in d.get('definitions',[]):
            if df['kind']=='OperationDefinition' and df.get('name'):
                docs.setdefault(df['name']['value'], (os.path.basename(f), df))
            elif df['kind']=='FragmentDefinition':
                frags.setdefault(df['name']['value'], (os.path.basename(f), df))
print('operations', len(docs), 'fragments', len(frags), 'unparsed', bad)
os.makedirs('ast', exist_ok=True)
def closure(d):
    need=set(); todo=list(spreads(d))
    while todo:
        n=todo.pop()
        if n in need: continue
        need.add(n)
        if n in frags: todo.extend(spreads(frags[n][1]))
    return need
for name,(f,d) in docs.items():
    need=closure(d)
    text='# chunk '+f+'\n'+p_def(d)+'\n'
    for n in sorted(need):
        text+='\n'+(p_def(frags[n][1]) if n in frags else f'# fragment {n} not found in the downloaded chunks')+'\n'
    open(f'ast/{name}.graphql','w').write(text)
for name in sorted(docs):
    if re.search(sys.argv[1] if len(sys.argv)>1 else 'networth|external|aggregator|household|liabilit', name, re.I): print(name, '<-', docs[name][0], 'fragments:', sorted(closure(docs[name][1])))
