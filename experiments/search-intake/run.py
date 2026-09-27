#!/usr/bin/env python3
"""Bounded Common Crawl hockey search-intake spike. Python stdlib only."""
from __future__ import annotations
import argparse, datetime as dt, email.utils, html, html.parser, json, os, re, subprocess, sys, time, urllib.error, urllib.parse, urllib.request, zlib
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent
CACHE = ROOT / ".tmp" / "search-intake"
os.environ["TMPDIR"] = str(ROOT / ".tmp")
PROXY = "http://127.0.0.1:8977"
MODEL = "@cf/meta/llama-3.1-8b-instruct-fp8"
PAGE_LIMIT = 300
MAX_BYTES = 2_000_000
SEEDS = [
"Anaheim Ducks","Boston Bruins","Buffalo Sabres","Calgary Flames","Carolina Hurricanes","Chicago Blackhawks","Colorado Avalanche","Columbus Blue Jackets","Dallas Stars","Detroit Red Wings","Edmonton Oilers","Florida Panthers","Los Angeles Kings","Minnesota Wild","Montreal Canadiens","Nashville Predators","New Jersey Devils","New York Islanders","New York Rangers","Ottawa Senators","Philadelphia Flyers","Pittsburgh Penguins","San Jose Sharks","Seattle Kraken","St. Louis Blues","Tampa Bay Lightning","Toronto Maple Leafs","Utah Mammoth","Vancouver Canucks","Vegas Golden Knights","Washington Capitals","Winnipeg Jets","National Hockey League","Professional Women's Hockey League","American Hockey League","International Ice Hockey Federation","Hockey Canada","ice hockey"
]
QUERIES = ["Edmonton Oilers","Calgary Flames","Toronto Maple Leafs","Montreal Canadiens","Vancouver Canucks","Boston Bruins","National Hockey League","hockey","Calgary hockey","NHL standings"]

def request(url, data=None, headers=None, retries=3, expect_range=None, timeout=25):
    req = urllib.request.Request(url, data=data, headers=headers or {}, method="POST" if data is not None else "GET")
    last = None
    for attempt in range(retries):
        try:
            with urllib.request.urlopen(req, timeout=timeout) as res:
                if expect_range is not None:
                    expected=f"bytes {expect_range[0]}-{expect_range[1]}/"
                    actual=res.headers.get("Content-Range","")
                    if res.status != 206 or not actual.startswith(expected):
                        raise RuntimeError(f"Common Crawl ignored or mismatched range ({res.status}, {actual[:80]!r})")
                return res.read()
        except urllib.error.HTTPError as e:
            last=e
            if e.code == 429:
                retry_after=e.headers.get("Retry-After")
                if not retry_after or attempt == retries-1: raise
                time.sleep(max(1.0,float(retry_after)))
                continue
            if e.code not in (429, 500, 502, 503, 504) or attempt == retries-1: raise
            time.sleep(min(30, 2 ** attempt))
        except (TimeoutError, urllib.error.URLError) as e:
            last=e
            if attempt == retries-1: raise
            time.sleep(2 ** attempt)
    raise last

def timed(stages, name, fn):
    start=time.monotonic(); value=fn(); stages[name]=round(time.monotonic()-start,3); return value

def wikidata_seeds():
    # Resolve requested page names through Wikipedia sitelinks, then read the
    # authoritative seed properties from Wikidata's batched entity API.
    titles="|".join(SEEDS)
    wiki_url="https://en.wikipedia.org/w/api.php?"+urllib.parse.urlencode({"action":"query","titles":titles,"prop":"pageprops","ppprop":"wikibase_item","redirects":"1","format":"json"})
    headers={"User-Agent":"zega-search-intake-spike/1.0 (https://github.com/zegadb/zega)","Accept":"application/json"}
    wiki_raw=request(wiki_url,headers=headers,retries=1,timeout=25)
    (CACHE/"wikipedia-pageprops.json").write_bytes(wiki_raw)
    pages=json.loads(wiki_raw)["query"]["pages"].values()
    by_title={p.get("title",""):p.get("pageprops",{}).get("wikibase_item") for p in pages}
    # Redirect resolution may change the displayed title; preserve input order
    # by matching normalized page titles case-insensitively as a fallback.
    qid_by_seed={}
    for seed in SEEDS:
        qid=by_title.get(seed)
        if not qid:
            qid=next((v for title,v in by_title.items() if title.casefold()==seed.casefold()),None)
        if qid: qid_by_seed[seed]=qid
    qids=list(dict.fromkeys(qid_by_seed.values()))
    entity_raw=b"{}"
    if qids:
        entity_url="https://www.wikidata.org/w/api.php?"+urllib.parse.urlencode({"action":"wbgetentities","ids":"|".join(qids),"props":"labels|aliases|claims","languages":"en","format":"json"})
        entity_raw=request(entity_url,headers=headers,retries=1,timeout=25)
    (CACHE/"wikidata-entities.json").write_bytes(entity_raw)
    records=json.loads(entity_raw).get("entities",{})
    def values(record,prop):
        out=[]
        for claim in record.get("claims",{}).get(prop,[]):
            mainsnak=claim.get("mainsnak",{})
            if mainsnak.get("snaktype")!="value": continue
            value=mainsnak.get("datavalue",{}).get("value")
            if isinstance(value,dict) and "id" in value: out.append(value["id"])
            elif isinstance(value,str): out.append(value)
        return list(dict.fromkeys(out))
    entities=[]; missing=[]
    for seed in SEEDS:
        qid=qid_by_seed.get(seed); record=records.get(qid,{}) if qid else {}
        label=record.get("labels",{}).get("en",{}).get("value",seed)
        if not qid or not record:
            missing.append(seed); continue
        aliases=[x.get("value","") for x in record.get("aliases",{}).get("en",[]) if x.get("value")]
        related=values(record,"P118")+values(record,"P641")
        entities.append({"id":qid,"label":label,"official_site":next(iter(values(record,"P856")),""),"kind":"|".join(values(record,"P31")),"aliases":list(dict.fromkeys(aliases)),"related_qids":list(dict.fromkeys(related))})
    return entities,missing

class Extract(html.parser.HTMLParser):
    def __init__(self): super().__init__(convert_charrefs=True); self.title=False; self.skip=0; self.main_depth=0; self.parts=[]; self.main_parts=[]; self.title_parts=[]; self.links=[]
    def handle_starttag(self,tag,attrs):
        attrs=dict(attrs)
        if tag in ("script","style","nav","footer","header","noscript","svg"): self.skip+=1
        if tag in ("main","article"): self.main_depth+=1
        if tag=="title": self.title=True
        if tag=="a" and attrs.get("href"): self.links.append(attrs["href"])
    def handle_endtag(self,tag):
        if tag in ("script","style","nav","footer","header","noscript","svg") and self.skip: self.skip-=1
        if tag in ("main","article") and self.main_depth: self.main_depth-=1
        if tag=="title": self.title=False
    def handle_data(self,data):
        value=re.sub(r"\s+"," ",data).strip()
        if value and self.skip==0 and not self.title:
            self.parts.append(value)
            if self.main_depth: self.main_parts.append(value)
        if value and self.title: self.title_parts.append(value)

def parse_record(raw, url, stamp):
    # WARC headers are ASCII; payload can use any declared page encoding.
    head,_,body=raw.partition(b"\r\n\r\n")
    if not body: return None
    charset=re.search(rb"charset\s*=\s*[\"']?([\w-]+)",head,re.I)
    encoding=charset.group(1).decode("ascii","ignore") if charset else "utf-8"
    try: source=body.decode(encoding,errors="replace")
    except LookupError: source=body.decode("utf-8",errors="replace")
    parser=Extract()
    try: parser.feed(source)
    except Exception: pass
    body_parts=parser.main_parts if parser.main_parts else parser.parts
    text=re.sub(r"\s+"," "," ".join(body_parts)).strip()[:12000]
    title=" ".join(parser.title_parts).strip()[:500]
    if not text: return None
    host=urllib.parse.urlsplit(url).hostname or ""
    language="en" if re.search(r"\b(the|and|hockey|team|league)\b",text[:2000],re.I) else "unknown"
    links=[]
    for href in parser.links:
        resolved=urllib.parse.urljoin(url,href)
        if urllib.parse.urlsplit(resolved).scheme in ("http","https") and resolved not in links: links.append(resolved)
        if len(links)>=80: break
    return {"url":url,"host":host,"title":title,"text":text,"outlinks":links,"language":language,"fetch_date":stamp}

def fetch_pages(entities,limit):
    collinfo_path=CACHE/"collinfo.json"
    collinfo=collinfo_path.read_bytes() if collinfo_path.exists() else request("https://index.commoncrawl.org/collinfo.json",headers={"User-Agent":"zega-search-intake-spike/1.0"})
    collinfo_path.write_bytes(collinfo)
    info=json.loads(collinfo)
    crawl=info[0]["id"]; index=info[0]["cdx-api"]
    (CACHE/"crawl.json").write_text(json.dumps({"id":crawl,"index":index},indent=2))
    pages=[]; hits=0; seen=set(); warc_bytes=0; fetched=0; downloaded=0; cdx_seconds=0.0; warc_seconds=0.0; parse_seconds=0.0; reused=0; fetch_errors=[]
    error_cache=CACHE/"cdx-errors.json"
    try: cached_errors=json.loads(error_cache.read_text())
    except (OSError,json.JSONDecodeError): cached_errors=[]
    def remember_cdx_error(host,error):
        item={"host":host,"error":error}
        if item not in cached_errors: cached_errors.append(item)
        error_cache.write_text(json.dumps(cached_errors,ensure_ascii=False,indent=2)+"\n")
    # Bound candidates per host; only exact official host, no page crawling.
    hosts=[]
    for e in entities:
        u=e["official_site"]
        host=urllib.parse.urlsplit(u).hostname if u else None
        if host and host not in hosts: hosts.append(host)
    per_host=max(1,min(12,(limit+len(hosts)-1)//max(len(hosts),1)))
    last_request=0.0
    for host in hosts:
        if fetched>=limit: break
        safe_host=re.sub(r"[^a-zA-Z0-9.-]","_",host)
        cdx_path=CACHE/("cdx-"+safe_host+".jsonl")
        if cdx_path.exists():
            body=cdx_path.read_bytes()
            if not body:
                prior=next((x["error"] for x in cached_errors if x.get("host")==host),None)
                fetch_errors.append(prior or f"CDX returned no records for {host}; an earlier latest-crawl lookup failed")
        else:
            wait=1.0-(time.monotonic()-last_request)
            if wait>0: time.sleep(wait)
            params=urllib.parse.urlencode({"url":host+"/*","output":"json","limit":"500","filter":["status:200","mime:text/html"],"fl":"url,mime,status,filename,offset,length,timestamp","collapse":"urlkey"},doseq=True)
            last_request=time.monotonic()
            cdx_start=time.monotonic()
            try: body=request(index+"?"+params,headers={"User-Agent":"zega-search-intake-spike/1.0 (Common Crawl URL index research)"})
            except Exception as e:
                error=f"CDX host failed {host}: {type(e).__name__}: {e}"
                print(error,file=sys.stderr); fetch_errors.append(error); remember_cdx_error(host,error); cdx_path.write_bytes(b""); continue
            cdx_seconds+=time.monotonic()-cdx_start
            cdx_path.write_bytes(body)
        candidates=[]
        for line in body.splitlines():
            try:
                row=json.loads(line); url=row.get("url","")
                if url and url not in seen and int(row.get("length",0))<=5_000_000: candidates.append(row); seen.add(url)
            except (ValueError,TypeError): continue
        hits+=len(candidates)
        # Favor likely landing pages and cap. Each next record is an independent Range request.
        candidates.sort(key=lambda r:(url_depth(r.get("url","")),len(r.get("url",""))))
        for row in candidates[:per_host]:
            if fetched>=limit: break
            start=int(row["offset"]); end=start+int(row["length"])-1
            warc_url=row["filename"]
            if not urllib.parse.urlsplit(warc_url).scheme:
                warc_url="https://data.commoncrawl.org/"+warc_url.lstrip("/")
            warc_dir=CACHE/"warc"; warc_dir.mkdir(exist_ok=True)
            cache_file=warc_dir/(str(row.get("timestamp",""))+"-"+str(start)+".gz")
            if cache_file.exists():
                raw=cache_file.read_bytes(); reused+=1
            else:
                wait=1.0-(time.monotonic()-last_request)
                if wait>0: time.sleep(wait)
                last_request=time.monotonic()
                warc_start=time.monotonic()
                try: raw=request(warc_url,headers={"Range":f"bytes={start}-{end}","User-Agent":"zega-search-intake-spike/1.0"},expect_range=(start,end))
                except Exception as e:
                    error=f"WARC record failed {row.get('url')}: {type(e).__name__}: {e}"
                    print(error,file=sys.stderr); fetch_errors.append(error); continue
                warc_seconds+=time.monotonic()-warc_start
                cache_file.write_bytes(raw)
                downloaded+=1
            fetched+=1
            warc_bytes+=len(raw)
            # WARC range commonly yields gzip member(s), decompress one member only.
            parse_start=time.monotonic()
            try: record=zlib.decompress(raw,16+zlib.MAX_WBITS)
            except zlib.error:
                parse_seconds+=time.monotonic()-parse_start; continue
            marker=record.find(b"\r\n\r\n",record.find(b"\r\n\r\n")+4)
            if marker>=0:
                http=record[marker+4:]
                page=parse_record(http,row["url"],row.get("timestamp", ""))
                if page: pages.append(page)
            parse_seconds+=time.monotonic()-parse_start
    return crawl,pages,hits,warc_bytes,len(hosts),fetched,downloaded,cdx_seconds,warc_seconds,parse_seconds,reused,fetch_errors

def url_depth(u): return len([x for x in urllib.parse.urlsplit(u).path.split("/") if x])

def exact_link(page,entities):
    hay=(page["title"]+" "+page["text"][:5000]).casefold()
    matches=[]
    for e in entities:
        terms=[e["label"],*e["aliases"]]
        if any(re.search(r"(?<!\w)"+re.escape(t.casefold())+r"(?!\w)",hay) for t in terms if len(t)>=3): matches.append(e)
    return matches

def classify(page,matches,entities):
    if len(matches)==1:return matches[0],"exact_label_alias",1.0
    names=[e["label"] for e in entities]
    prompt={"url":page["url"],"title":page["title"],"text":page["text"][:1800],"candidate_entities":names}
    payload={"messages":[{"role":"system","content":"Choose the single hockey entity most directly covered by this web page. Return JSON only: {\"entity\": exact candidate label or null, \"confidence\": number 0..1}. Do not infer from web host alone."},{"role":"user","content":json.dumps(prompt,ensure_ascii=False)}],"max_tokens":120,"temperature":0}
    raw=request(PROXY+"/run/"+urllib.parse.quote(MODEL,safe="@/"),json.dumps(payload).encode(),{"Content-Type":"application/json"})
    data=json.loads(raw); content=data.get("result",{}).get("response","")
    m=re.search(r"\{.*?\}",content,re.S)
    if not m: return None,"workers_ai_unparsed",0.0
    try: answer=json.loads(m.group())
    except json.JSONDecodeError:return None,"workers_ai_unparsed",0.0
    label=answer.get("entity"); conf=answer.get("confidence",0)
    if label not in names:return None,"workers_ai",0.0
    return next(e for e in entities if e["label"]==label),"workers_ai",float(conf)

def zqlstr(value): return json.dumps(value,ensure_ascii=False)
def build_data(entities,pages,linked):
    entities_doc=[{"qid":e["id"],"label":e["label"],"aliases":" | ".join(e["aliases"][:20]),"kind":e["kind"],"official_site":e["official_site"]} for e in entities]
    page_doc=[]
    for i,(p,e,method,conf) in enumerate(linked):
        page_doc.append({"url":p["url"],"host":p["host"],"title":p["title"] or p["url"],"text":p["text"][:4000],"language":p["language"],"fetch_date":p["fetch_date"],"entity_qid":e["id"] if e else "","link_method":method,"confidence":conf,"outlinks":"\n".join(p["outlinks"][:40])})
    (OUT/"entities.json").write_text(json.dumps(entities_doc,ensure_ascii=False,indent=2))
    (OUT/"pages.json").write_text(json.dumps(page_doc,ensure_ascii=False,indent=2))
    schema='''schema {
 type Entity { qid: String label: String aliases: String kind: String official_site: String RELATED_TO -> Entity[] ABOUT <- Page[] }
 type Page { url: String host: String title: String text: String language: String fetch_date: String entity_qid: String link_method: String confidence: Float outlinks: String LINKS_TO -> Page[] ABOUT -> Entity }
} unique { Entity { qid } Page { url } }'''
    (OUT/"schema.zql").write_text(schema+"\n")
    return schema,entities_doc,page_doc

def load_files(schema,server_url,entities,entities_doc,page_doc):
    # load in bounded <=2 MB chunks as engine import source limit applies per source
    batches=[]
    for typ,rows,cols in (("Entity",entities_doc,["qid","label","aliases","kind","official_site"]),("Page",page_doc,["url","host","title","text","language","fetch_date","entity_qid","link_method","confidence","outlinks"])):
        chunk=[]
        for row in rows:
            candidate=chunk+[row]
            if len(json.dumps(candidate,ensure_ascii=False).encode())>1_800_000 and chunk:
                batches.append((typ,chunk,cols)); chunk=[row]
            else: chunk=candidate
        if chunk:batches.append((typ,chunk,cols))
    total=0
    for ix,(typ,rows,cols) in enumerate(batches):
        fname=f"load-{ix:03}.json"; (OUT/fname).write_text(json.dumps(rows,ensure_ascii=False))
        fields=" ".join(cols)
        expr=f'mutation json ["experiments/search-intake/{fname}"] {{ {typ}({" && ".join(f"{c}: ${c}" for c in cols)}) {{ {fields} }} }}'
        # First import creates nodes; edge linking is a second mutation using entity_qid/url selectors.
        payload={"schema":schema,"query":expr}
        try:
            raw=request(server_url+"/zql",json.dumps(payload).encode(),{"Content-Type":"application/json"})
        except urllib.error.HTTPError as exc:
            detail=exc.read().decode("utf-8","replace")[:3000]
            raise RuntimeError(f"zega {typ} batch {ix} load returned HTTP {exc.code}: {detail}") from exc
        result=json.loads(raw)
        if not result.get("ok"): raise RuntimeError("zega load failed: "+str(result.get("error")))
        total+=len(rows)
    # ABOUT entity links: explicit link to existing entity using the stored QID.
    linked_count=0
    for row in page_doc:
        if not row["entity_qid"]: continue
        expr=f'mutation {{ Page(url: {zqlstr(row["url"])}) {{ ABOUT -> link Entity(qid: {zqlstr(row["entity_qid"])}) {{ qid }} }} }}'
        result=json.loads(request(server_url+"/zql",json.dumps({"schema":schema,"query":expr}).encode(),{"Content-Type":"application/json"}))
        if not result.get("ok"): raise RuntimeError("ABOUT linking failed: "+str(result.get("error")))
        linked_count+=1
    # Preserve the Wikidata team -> league and league -> sport links.
    qids={e["qid"] for e in entities_doc}
    for e in entities:
        for qid in e.get("related_qids",[]):
            if qid not in qids: continue
            expr=f'mutation {{ Entity(qid: {zqlstr(e["id"])}) {{ RELATED_TO -> link Entity(qid: {zqlstr(qid)}) {{ qid }} }} }}'
            result=json.loads(request(server_url+"/zql",json.dumps({"schema":schema,"query":expr}).encode(),{"Content-Type":"application/json"}))
            if not result.get("ok"): raise RuntimeError("Wikidata relationship failed: "+str(result.get("error")))
            linked_count+=1
    # Resolve captured outlinks to captured pages and insert graph-native link edges.
    by_url={r["url"]:r for r in page_doc}; pages_by_host=defaultdict(list)
    for url in by_url: pages_by_host[urllib.parse.urlsplit(url).hostname].append(url)
    for source in page_doc:
        targets=set()
        for target in source["outlinks"].splitlines():
            if target in by_url and target!=source["url"]: targets.add(target)
        # Normalize common canonical URL differences without inventing uncaptured nodes.
        for target in list(targets):
            expr=f'mutation {{ Page(url: {zqlstr(source["url"])}) {{ LINKS_TO -> link Page(url: {zqlstr(target)}) {{ url }} }} }}'
            result=json.loads(request(server_url+"/zql",json.dumps({"schema":schema,"query":expr}).encode(),{"Content-Type":"application/json"}))
            if not result.get("ok"): raise RuntimeError("LINKS_TO linking failed: "+str(result.get("error")))
            linked_count+=1
    return total,linked_count

def query(server_url,schema,q):
    # literal substring search scored by text/title and incoming links to page URLs
    words=[w for w in re.findall(r"[\w'-]+",q.casefold()) if len(w)>2]
    expr="{ Page { url title host text entity_qid link_method confidence LINKS_TO -> Page { url } } }"
    data=json.loads(request(server_url+"/zql",json.dumps({"schema":schema,"query":expr}).encode(),{"Content-Type":"application/json"}))
    if not data.get("ok"): raise RuntimeError("search graph read failed: "+str(data.get("error")))
    result=data.get("result",[])
    pages=[]
    # Zega returns one dictionary per matched root node (not a type-key wrapper).
    incoming=Counter()
    for item in result:
        if not isinstance(item,dict) or "url" not in item: continue
        links=item.get("LINKS_TO",[])
        if isinstance(links,dict): links=[links]
        for link in links: incoming[link.get("url")]+=1
        pages.append(item)
    for p in pages:
        text=(str(p.get("title",""))+" "+str(p.get("text",""))).casefold()
        p["text_score"]=sum(text.count(w) for w in words)
        p["link_score"]=incoming[p.get("url")]
        p["score"]=p["text_score"]+p["link_score"]
    return sorted((p for p in pages if p["score"]>0),key=lambda p:(-p["score"],p.get("url","")))[:10]

page_urls=[]
def main():
    ap=argparse.ArgumentParser(); ap.add_argument("--limit",type=int,default=300); args=ap.parse_args()
    if not 1<=args.limit<=PAGE_LIMIT: ap.error("--limit must be in 1..300")
    CACHE.mkdir(parents=True,exist_ok=True)
    stages={}; failures=[]; t0=time.monotonic()
    entities,missing=timed(stages,"wikidata_seconds",wikidata_seeds)
    if not entities: raise RuntimeError("Wikidata returned no seed entities")
    crawl,pages,cdx_hits,warc_bytes,hosts,fetched,downloaded,cdx_s,warc_s,parse_s,reused,fetch_errors=timed(stages,"common_crawl_seconds",lambda:fetch_pages(entities,args.limit))
    failures.extend(fetch_errors)
    stages["cdx_seconds"]=round(cdx_s,3); stages["warc_fetch_seconds"]=round(warc_s,3); stages["parse_extract_seconds"]=round(parse_s,3)
    exact_counts=Counter(); linked=[]; model_calls=0; ambiguous_count=0
    classify_start=time.monotonic()
    for p in pages:
        matches=exact_link(p,entities)
        if len(matches)==1: e,method,confidence=matches[0],"exact_label_alias",1.0
        else:
            if len(matches)>1: ambiguous_count+=1
            model_calls+=1
            try: e,method,confidence=classify(p,matches,entities)
            except Exception as exc: e,method,confidence=None,"workers_ai_error",0.0; failures.append(f"Workers AI {p['url']}: {type(exc).__name__}: {exc}")
        exact_counts[method]+=1; linked.append((p,e,method,confidence))
    stages["classification_seconds"]=round(time.monotonic()-classify_start,3)
    linked_stage=time.monotonic()
    schema,entities_doc,page_doc=build_data(entities,pages,linked)
    stages["classify_parse_save_seconds"]=round(time.monotonic()-linked_stage,3)
    # run local zega service bound to loopback with per-task data, record pid and stop only our Popen child
    data_dir=CACHE/f"database-{os.getpid()}"; data_dir.mkdir(exist_ok=True)
    exe=ROOT/".target/release/zega"
    if not exe.exists(): exe=ROOT/".target/debug/zega"
    if not exe.exists(): raise RuntimeError("build zega-cli first (cargo build --locked -p zega-cli)")
    server=subprocess.Popen([str(exe),"start","--data",str(data_dir),"--port","0"],cwd=ROOT,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    try:
        line=server.stdout.readline().strip()
        m=re.search(r"http://127\.0\.0\.1:(\d+)",line)
        if not m: raise RuntimeError(f"could not read server address (pid {server.pid}): {line}")
        base="http://127.0.0.1:"+m.group(1)
        deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            try: request(base+"/health"); break
            except Exception: time.sleep(.1)
        start=time.monotonic(); loaded,about=load_files(schema,base,entities,entities_doc,page_doc); stages["zega_load_seconds"]=round(time.monotonic()-start,3)
        stats=json.loads(request(base+"/stats")); stats=stats.get("result",{})
        try:
            pidstat=subprocess.run(["ps","-o","rss=","-p",str(server.pid)],capture_output=True,text=True,check=True).stdout.strip()
            memory_kib=int(pidstat) if pidstat else None
        except Exception: memory_kib=None
        global page_urls
        page_urls=[p["url"] for p in page_doc]
        searches=[]; stage=time.monotonic()
        for q in QUERIES:
            try: found=query(base,schema,q); searches.append({"query":q,"results":found})
            except Exception as e: failures.append(f"query {q}: {type(e).__name__}: {e}"); searches.append({"query":q,"results":[]})
        stages["search_seconds"]=round(time.monotonic()-stage,3)
        def registrable(host):
            parts=(host or "").casefold().split(".")
            return ".".join(parts[-2:]) if len(parts)>=2 else (host or "").casefold()
        referee=[]
        for e in entities:
            if not e["official_site"]: continue
            own=[s for s in searches if s["query"].casefold()==e["label"].casefold()]
            if not own: continue
            site_host=urllib.parse.urlsplit(e["official_site"]).hostname
            pos=next((i+1 for i,p in enumerate(own[0]["results"]) if p.get("entity_qid")==e["id"] and registrable(p.get("host"))==registrable(site_host)),None)
            referee.append({"entity":e["label"],"official_site":e["official_site"],"rank":pos,"top3":bool(pos and pos<=3)})
        skipped=fetched-len(pages)
        if skipped: failures.append(f"{skipped} of {fetched} fetched WARC records could not be decoded into nonempty HTML pages")
        if referee and not any(row["top3"] for row in referee): failures.append(f"Referee score was 0/{len(referee)}; no matching official-domain page ranked in the top three")
        if any("blog.mathspace.co" in p["url"] for p in pages): failures.append("Common Crawl returned a homepage variant with unrelated ref=blog.mathspace.co tracking text; it appeared in hockey search results")
        # Three query->entity->page graph paths, derived from loaded node values.
        transparency=[]
        for s in searches:
            if len(transparency)>=3:break
            if not s["results"]:continue
            query_text=s["query"].casefold()
            query_entities=[e for e in entities if any(re.search(r"(?<!\w)"+re.escape(term.casefold())+r"(?!\w)",query_text) for term in [e["label"],*e["aliases"]] if len(term)>=3)]
            if not query_entities: continue
            e=max(query_entities,key=lambda item:len(item["label"]))
            if not e:continue
            path_query=f'{{ Entity(qid: {zqlstr(e["id"])}) {{ label ABOUT <- Page {{ url title }} RELATED_TO -> Entity {{ label ABOUT <- Page {{ url title }} }} }} }}'
            path_result=json.loads(request(base+"/zql",json.dumps({"schema":schema,"query":path_query}).encode(),{"Content-Type":"application/json"}))
            path_rows=path_result.get("result",[])
            path_roots=[path_rows] if isinstance(path_rows,dict) else path_rows
            reachable={}
            for root in path_roots:
                if not isinstance(root,dict): continue
                for page in root.get("ABOUT",[]): reachable[page.get("url")]=(None,page)
                related=root.get("RELATED_TO",[])
                if isinstance(related,dict): related=[related]
                for middle in related:
                    for page in middle.get("ABOUT",[]): reachable[page.get("url")]=(middle.get("label"),page)
            match=next((p for p in s["results"] if p["url"] in reachable),None)
            if not match: continue
            middle_label,page=reachable[match["url"]]
            path=[{"type":"Query","text":s["query"],"edge":"text_match"},{"type":"Entity","qid":e["id"],"label":e["label"]}]
            if middle_label: path.extend([{"type":"Entity","label":middle_label,"edge":"RELATED_TO"}])
            path.append({"type":"Page","url":page["url"],"title":page["title"],"edge":"ABOUT (reverse traversal)"})
            transparency.append({"query":s["query"],"path":path,"zql":path_query,"verified_from_graph":True})
        cdx_times=[p.stat().st_mtime for p in CACHE.glob("cdx-*.jsonl") if p.exists()]
        warc_times=[p.stat().st_mtime for p in (CACHE/"warc").glob("*.gz") if p.exists()]
        results={"crawl":crawl,"seed_counts":{"requested":len(SEEDS),"found":len(entities),"missing":missing},"common_crawl":{"index_hits":cdx_hits,"hosts_queried":hosts,"warc_bytes_processed":warc_bytes,"fetched":fetched,"range_records_downloaded_this_run":downloaded,"warc_cache_records_reused":reused,"parsed":len(pages),"parse_or_decode_skipped":fetched-len(pages),"cache_mtime_span_seconds":{"cdx_metadata":round(max(cdx_times)-min(cdx_times),3) if len(cdx_times)>1 else 0.0,"warc_records":round(max(warc_times)-min(warc_times),3) if len(warc_times)>1 else 0.0},"cache":".tmp/search-intake"},"linked":{"total":len(linked),"methods":dict(exact_counts),"ambiguous_exact_matches":ambiguous_count,"workers_ai_model":MODEL,"workers_ai_calls":model_calls,"unlinked":sum(e is None for _,e,_,_ in linked)},"zega":{"loaded_nodes":loaded,"graph_edges_created":about,"nodes":stats.get("nodes"),"relationships":stats.get("relationships"),"load_seconds":stages["zega_load_seconds"],"process_rss_after_load_kib":memory_kib},"timings_seconds":stages,"total_seconds":round(time.monotonic()-t0,3),"referee":{"scored":len(referee),"top3":sum(x["top3"] for x in referee),"score":round(sum(x["top3"] for x in referee)/len(referee),3) if referee else None,"entities":referee},"queries":searches,"transparency_paths":transparency,"what_broke":failures}
        (OUT/"results.json").write_text(json.dumps(results,ensure_ascii=False,indent=2)+"\n")
    finally:
        server.terminate(); server.wait(timeout=10)
    print(json.dumps({"crawl":crawl,"seeds":len(entities),"cdx_hits":cdx_hits,"fetched":len(pages),"linked":dict(exact_counts),"results":str(OUT/"results.json")},ensure_ascii=False))

if __name__=="__main__":
    try: main()
    except Exception as exc:
        print(f"search intake failed: {type(exc).__name__}: {exc}",file=sys.stderr); raise
