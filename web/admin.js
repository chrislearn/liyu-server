'use strict';
const $ = s => document.querySelector(s);
let csrf = '', current = null, next = null, rows = [];
const categories = {coffee:'咖啡茶饮',movie:'电影演出',trendy:'潮流小物',blind:'盲盒',sweet:'甜点鲜花',digital:'数码家电',home:'家居生活',baby:'母婴亲子'};
async function api(path, options = {}) {
  const headers = {'X-Admin-Request':'1',...options.headers};
  if (csrf) headers['X-CSRF-Token'] = csrf;
  if (options.body && typeof options.body === 'string') headers['Content-Type'] = 'application/json';
  const response = await fetch('/admin/api/'+path,{...options,headers,credentials:'same-origin'});
  if (!response.ok) {
    if(response.status===401 && path!=='login'){showLogin();$('#editor').close();}
    const data = await response.json().catch(()=>({}));
    throw new Error(data.error || `请求失败（${response.status}）`);
  }
  return response.status===204 ? null : response.json();
}
function notice(message){$('#notice').textContent=message;}
function showLogin(){csrf='';$('#login-panel').hidden=false;$('#workspace').hidden=true;$('#identity').hidden=true;}
function showWorkspace(user){csrf=user.csrf_token;$('#username').textContent=user.username;$('#identity').hidden=false;$('#login-panel').hidden=true;$('#workspace').hidden=false;}
function cell(text){const c=document.createElement('td');c.textContent=text;return c;}
function render(){
  const body=$('#product-rows');body.replaceChildren();
  for(const p of rows){
    const tr=document.createElement('tr'), first=document.createElement('td'), img=document.createElement('img'), info=document.createElement('div'), name=document.createElement('span'), id=document.createElement('small');
    img.src=`/api/v1/media/products/${p.id}/thumb`;img.alt='';img.addEventListener('error',()=>img.hidden=true,{once:true});
    name.textContent=p.name;id.textContent=`#${p.id} · ${p.brand || '未设置品牌'}`;info.append(name,id);first.append(img,info);tr.append(first,cell(categories[p.category]||p.category),cell(`¥${(p.price_cents/100).toFixed(2)}`),cell(p.stock));
    const status=cell(''),badge=document.createElement('span');badge.className='badge'+(p.is_active?'':' off');badge.textContent=p.is_active?'已上架':'已下架';status.append(badge);tr.append(status);
    const actions=cell(''),edit=document.createElement('button'),toggle=document.createElement('button');edit.textContent='编辑';edit.className='quiet';edit.onclick=()=>openEditor(p);toggle.textContent=p.is_active?'下架':'上架';toggle.className='quiet';toggle.onclick=async()=>{toggle.disabled=true;try{await api(`products/${p.id}`,{method:'PUT',body:JSON.stringify({...p,is_active:!p.is_active})});await load();notice('商品状态已更新');}catch(e){notice(e.message);}finally{toggle.disabled=false;}};actions.append(edit,toggle);tr.append(actions);body.append(tr);
  }
  if(!rows.length){const tr=document.createElement('tr'),td=cell('暂无商品');td.colSpan=6;tr.append(td);body.append(tr);}
  $('#more').hidden=next===null;
}
async function load(append=false){const query=new URLSearchParams({q:$('#query').value,cursor:append?next:-1});const data=await api('products?'+query);rows=append?rows.concat(data.items):data.items;next=data.next_cursor;render();}
function openEditor(p){current=p?p.id:null;const f=$('#product-form');f.reset();$('#editor-title').textContent=p?'编辑商品':'新增商品';const defaults={name:'',category:'home',price_cents:0,stock:100,brand:'',kind:'',spec:'',description:'',tags:[],physical:true,is_active:true};const v=p||defaults;for(const k of ['name','category','stock','brand','kind','spec','description']) f.elements[k].value=v[k];f.elements.price.value=(v.price_cents/100).toFixed(2);f.elements.tags.value=v.tags.join(', ');for(const k of ['physical','is_active'])f.elements[k].checked=v[k];$('#editor-error').textContent='';$('#image-panel').hidden=!p;$('#image-file').value='';if(p)$('#preview').src=`/api/v1/media/products/${p.id}/card?t=${Date.now()}`;$('#editor').showModal();}
$('#login-form').onsubmit=async e=>{e.preventDefault();const f=e.target,b=f.querySelector('button');b.disabled=true;try{const user=await api('login',{method:'POST',body:JSON.stringify({username:f.elements.username.value,password:f.elements.password.value})});f.elements.password.value='';showWorkspace(user);await load();notice('');}catch(e){notice(e.message);}finally{b.disabled=false;}};
$('#logout').onclick=async()=>{try{await api('logout',{method:'POST'});showLogin();notice('已退出登录');}catch(e){notice(e.message);}};
$('#search-form').onsubmit=e=>{e.preventDefault();load().catch(e=>notice(e.message));};
$('#more').onclick=()=>load(true).catch(e=>notice(e.message));
$('#new-product').onclick=()=>openEditor(null);
$('#close-editor').onclick=()=>$('#editor').close();
$('#product-form').onsubmit=async e=>{
 e.preventDefault();const f=e.target,b=$('#save');b.disabled=true;
 const data={};for(const k of ['name','category','brand','kind','spec','description'])data[k]=f.elements[k].value;
 data.price_cents=Math.round(Number(f.elements.price.value)*100);data.stock=Number(f.elements.stock.value);data.tags=f.elements.tags.value.split(/[,，]/).map(t=>t.trim()).filter(Boolean);for(const k of ['physical','is_active'])data[k]=f.elements[k].checked;
 try{const result=await api(current===null?'products':`products/${current}`,{method:current===null?'POST':'PUT',body:JSON.stringify(data)});current=result.id;$('#editor-title').textContent='编辑商品';$('#image-panel').hidden=false;$('#editor-error').textContent='已保存，可继续上传图片。';await load();notice('商品已保存');}catch(e){$('#editor-error').textContent=e.message;}finally{b.disabled=false;}
};
$('#image-form').onsubmit=async e=>{e.preventDefault();const file=$('#image-file').files[0],b=e.target.querySelector('button');if(!file||current===null)return;if(file.size>4*1024*1024){$('#editor-error').textContent='图片不能超过 4 MiB';return;}b.disabled=true;try{const result=await api(`products/${current}/images/${$('#variant').value}`,{method:'POST',body:file,headers:{'Content-Type':file.type}});$('#preview').src=result.image_url+'?t='+Date.now();$('#editor-error').textContent='图片已上传';await load();}catch(e){$('#editor-error').textContent=e.message;}finally{b.disabled=false;}};
api('me').then(async user=>{showWorkspace(user);await load();}).catch(e=>{showLogin();if(!e.message.includes('administrator login required'))notice(e.message);});
