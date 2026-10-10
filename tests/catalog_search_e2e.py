#!/usr/bin/env python3
"""Read-only catalog tool contract; temporary rows on a disposable database only."""
import json,os,sys,subprocess,urllib.request,urllib.parse,urllib.error
base=sys.argv[1];dsn=os.environ['DATABASE_URL'];assert 'ai_search_test' in dsn
checks=0
def check(v,name):
 global checks
 assert v,name
 checks+=1
def call(path,status=200):
 try:r=urllib.request.urlopen(base+path)
 except urllib.error.HTTPError as e:r=e
 check(r.status==status,path+' status');return json.load(r)
def sql(s):subprocess.run(['psql',dsn,'-v','ON_ERROR_STOP=1','-c',s],check=True,capture_output=True)
f={'max_price_cents':3000,'allowed_kinds':['咖啡'],'excluded_kinds':['奶茶'],'delivery':'any'}
def search(filter=None,**params):return call('/api/v1/catalog/search?'+urllib.parse.urlencode({'filter':json.dumps(filter or f,ensure_ascii=False),**params}))
o=call('/api/v1/catalog/search-options');check(o['version']==1 and '咖啡' in o['kinds'] and '奶茶' in o['kinds'],'exact kinds');check(o['tool']['constraints_schema']['additionalProperties'] is False,'strict schema')
d=call('/api/v1/catalog/openapi');check(set(d['paths'])=={'/api/v1/catalog/search','/api/v1/catalog/search-options'},'read-only doc');check(d['components']['schemas']['SearchConstraints']['required']==['max_price_cents','allowed_kinds','excluded_kinds','delivery'],'constraints documented')
a=search();check(a['eligible_total']==0 and not a['items'] and a['next_cursor'] is None,'full catalog no match proof')
f['max_price_cents']=3500;a=search();check(a['eligible_total']==1 and [p['id'] for p in a['items']]==[1],'coffee not milk tea')
for q in ['not-a-real-brand','%',"' OR 1=1 --"]:
 a=search(q=q);check(a['eligible_total']==1 and a['total_matches']==0 and not a['items'],'soft emptiness not hard emptiness')
a=search(cursor=99999);check(a['eligible_total']==1 and not a['items'],'cursor emptiness not no match')
for changes in [{'max_price_cents':-1},{'delivery':'teleport'},{'allowed_kinds':['invented']},{'allowed_kinds':['咖啡','咖啡']},{'excluded_kinds':['咖啡']},{'ignore_stock':True}]:
 call('/api/v1/catalog/search?'+urllib.parse.urlencode({'filter':json.dumps({**f,**changes},ensure_ascii=False)}),400)
for key,value in [('cursor','wat'),('limit','0'),('limit','21'),('q','x'*101)]:call('/api/v1/catalog/search?'+urllib.parse.urlencode({'filter':json.dumps(f),key:value}),400)
revision=search()['catalog_revision'];sql('UPDATE catalog SET stock=0 WHERE id=1')
a=search();check(a['eligible_total']==0 and a['catalog_revision']!=revision,'stock and revision reflected');sql('UPDATE catalog SET stock=100 WHERE id=1')
try:
 sql("INSERT INTO catalog(id,name,category,price_cents,physical,brand,kind,spec,description,tags,stock,is_active) SELECT n,'分页测试咖啡'||n,'coffee',3400,false,'测试','咖啡','测试规格','测试商品',ARRAY[]::text[],10,true FROM generate_series(100,159) n")
 seen=[];cursor=-1;revision=''
 while True:
  a=search(cursor=cursor,limit=20);check(a['eligible_total']==61 and a['applied']==f,'full count independent of page');check(not revision or revision==a['catalog_revision'],'stable snapshot');revision=a['catalog_revision'];seen.extend(p['id'] for p in a['items'])
  if a['next_cursor'] is None:break
  cursor=a['next_cursor']
 check(len(seen)==61 and len(set(seen))==61 and 159 in seen,'pagination covers beyond first 50')
 a=search(q='分页测试咖啡159');check([p['id'] for p in a['items']]==[159],'server query finds late product')
 f['delivery']='physical';check(search()['eligible_total']==0,'delivery cannot broaden');f['delivery']='any'
finally:sql('DELETE FROM catalog WHERE id BETWEEN 100 AND 159')
print('CATALOG_TOOL_HTTP_CHECKS='+str(checks))
