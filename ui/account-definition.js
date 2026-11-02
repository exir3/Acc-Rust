/* WebApp-inspired account-definition popup for the Rust/Tauri client. */
(() => {
  'use strict';
  const invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
  const FA = '۰۱۲۳۴۵۶۷۸۹';
  const fa = v => String(v == null ? '' : v).replace(/[0-9]/g, d => FA[Number(d)]);
  const en = v => String(v == null ? '' : v).replace(/[۰-۹]/g, c => String(c.charCodeAt(0)-0x06f0)).replace(/[٠-٩]/g, c => String(c.charCodeAt(0)-0x0660));
  const nums = v => en(v).replace(/[^0-9]/g,'');
  const searchText = v => en(v).replace(/[يى]/g,'ی').replace(/ك/g,'ک').replace(/\u200c/g,'').replace(/\s+/g,' ').trim().toLowerCase();
  const esc = v => String(v == null ? '' : v).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const levelText = n => ({1:'کل',2:'معین',3:'تفصیلی ۱',4:'تفصیلی ۲ (قدیمی)'})[Number(n)] || '—';
  const natureText = n => ({debtor:'بدهکار',creditor:'بستانکار',dual:'دوگانه'})[n] || '—';

  function openAccountDefinitionPopup() {
    if (!invoke) { window.alert('این پنجره فقط در نسخه دسکتاپ قابل استفاده است.'); return; }
    if (document.querySelector('.adp2-overlay')) return;
    const state = { accounts:[], groups:[], selected:'', path:[], mode:'view', busy:false, search:'', group:'', level:'' };
    const overlay = document.createElement('div');
    overlay.className = 'adp2-overlay';
    overlay.innerHTML =
      '<section class="adp2-window" role="dialog" aria-modal="true" aria-labelledby="adp2-title" dir="rtl">' +
      '<header class="adp2-titlebar"><span>📁</span><strong id="adp2-title">تعریف حساب‌های کل، معین و تفصیلی</strong><button type="button" data-close aria-label="بستن">×</button></header>' +
      '<div class="adp2-content">' +
        '<nav class="adp2-breadcrumb" data-breadcrumb aria-label="مسیر حساب‌ها"></nav>' +
        '<div class="adp2-filters">' +
          '<label>جست‌وجو<input type="search" data-search placeholder="کد، نام یا حساب بالاتر"></label>' +
          '<label>گروه حساب<select data-group><option value="">همه گروه‌ها</option></select></label>' +
          '<label>سطح<select data-level><option value="">همه سطوح</option><option value="1">کل</option><option value="2">معین</option><option value="3">تفصیلی ۱</option><option value="4">تفصیلی ۲ (قدیمی)</option></select></label>' +
          '<button type="button" class="adp2-button" data-reset>پاک‌کردن فیلتر</button><button type="button" class="adp2-button" data-refresh>↻ بارگذاری مجدد</button>' +
        '</div>' +
        '<div class="adp2-grid-wrap"><table class="adp2-grid"><thead><tr><th>ردیف</th><th>کد حساب</th><th>نام حساب</th><th>سطح</th><th>گروه حساب</th><th>ماهیت</th><th>حساب بالاتر</th><th>ثبت سند</th></tr></thead><tbody data-rows><tr><td colspan="8">در حال بارگذاری…</td></tr></tbody></table></div>' +
        '<section class="adp2-editor" data-editor hidden><div class="adp2-editor-head"><b data-editor-title>مشخصات حساب</b><span data-editor-hint></span></div>' +
          '<form data-form class="adp2-form" autocomplete="off">' +
            '<label>کد حساب<input data-code dir="ltr" inputmode="numeric" maxlength="20" required></label>' +
            '<label class="adp2-wide">نام حساب<input data-name maxlength="255" required></label>' +
            '<label>سطح حساب<select data-edit-level><option value="1">کل</option><option value="2">معین</option><option value="3">تفصیلی ۱</option></select></label>' +
            '<label>گروه حساب<select data-edit-group required></select></label>' +
            '<label>حساب بالاتر<select data-parent></select></label>' +
            '<label>ماهیت<select data-nature><option value="debtor">بدهکار</option><option value="creditor">بستانکار</option><option value="dual">دوگانه</option></select></label>' +
            '<label class="adp2-check"><input type="checkbox" data-postable> قابل ثبت در سند</label>' +
            '<div class="adp2-form-error" data-error hidden></div><div class="adp2-form-actions">' +
              '<button type="submit" class="adp2-button primary" data-save>ذخیره (F2)</button><button type="button" class="adp2-button" data-cancel>انصراف</button>' +
            '</div></form></section>' +
      '</div><footer class="adp2-toolbar">' +
        '<button type="button" class="adp2-button" data-new>＋ حساب جدید</button><button type="button" class="adp2-button" data-edit>✎ اصلاح</button>' +
        '<button type="button" class="adp2-button danger" data-delete>حذف حساب</button><button type="button" class="adp2-button danger" data-delete-all>حذف همه</button>' +
        '<span class="adp2-spacer"></span><button type="button" class="adp2-button" data-export>خروجی اکسل</button><button type="button" class="adp2-button" data-import>ورودی اکسل</button><span class="adp2-count" data-count></span>' +
      '</footer><div class="adp2-status" data-status role="status"></div></section>';
    document.body.appendChild(overlay);

    const $ = s => overlay.querySelector(s);
    const rows = $('[data-rows]'), editor = $('[data-editor]'), form = $('[data-form]');
    const code = $('[data-code]'), name = $('[data-name]'), level = $('[data-edit-level]');
    const group = $('[data-edit-group]'), parent = $('[data-parent]'), nature = $('[data-nature]');
    const postable = $('[data-postable]'), errorBox = $('[data-error]'), status = $('[data-status]');
    let initialEditorValues = null;
    const editorValues = () => JSON.stringify([
      code.value, name.value, level.value, group.value, parent.value, nature.value, postable.checked,
    ]);
    const rememberEditorValues = () => { initialEditorValues = editorValues(); };
    const say = (message,tone) => { status.textContent = message || ''; status.dataset.tone = tone || ''; };
    const getSelected = () => state.accounts.find(a => a.code === state.selected) || null;
    const children = c => state.accounts.filter(a => a.parentCode === c);
    const findPath = account => {
      const path=[], seen=new Set(); let item=account;
      while(item && !seen.has(item.code)){seen.add(item.code);path.unshift(item.code);item=item.parentCode?state.accounts.find(a=>a.code===item.parentCode):null;}
      return path;
    };
    const newCode = (n,p) => {
      const siblings=state.accounts.filter(a=>Number(a.level)===n && (n===1?!a.parentCode:a.parentCode===p));
      const prefix=n===1?'':p, used=new Set(state.accounts.map(a=>String(a.code)));
      const suffixes=siblings.map(a=>nums(String(a.code).slice(prefix.length))).filter(Boolean).map(BigInt);
      let width=Math.max(n===1?4:2,...siblings.map(a=>String(a.code).length-prefix.length));
      const maxWidth=20-prefix.length;
      let value=suffixes.length?suffixes.reduce((maximum,suffix)=>suffix>maximum?suffix:maximum,0n)+1n
        :(n===1?10n**BigInt(width-1):1n);
      while(width<=maxWidth){
        const max=10n**BigInt(width)-1n;
        if(value<=max){
          while(value<=max){
            const candidate=prefix+value.toString().padStart(width,'0');
            if(!used.has(candidate))return candidate;
            value++;
          }
        }
        width++;
        value=n===1?10n**BigInt(width-1):1n;
      }
      return '';
    };
    const fillGroups = selected => {
      const options=state.groups.map(g=>'<option value="'+esc(g.code)+'">'+fa(g.code)+' — '+esc(g.name)+' ('+esc(g.type)+')</option>').join('');
      $('[data-group]').innerHTML='<option value="">همه گروه‌ها</option>'+options;
      group.innerHTML=options; if(selected) group.value=selected;
      $('[data-group]').value=state.group;
    };
    const filtered = () => {
      const q=searchText(state.search), hasFilter=!!(q||state.group||state.level);
      const current=state.path[state.path.length-1]||'';
      return state.accounts.filter(a=>{
        if(!hasFilter && (current?a.parentCode!==current:Number(a.level)!==1)) return false;
        if(state.group && a.groupCode!==state.group) return false;
        if(state.level && Number(a.level)!==Number(state.level)) return false;
        const hay=searchText([a.code,a.name,a.parentName,a.groupName].join(' '));
        return !q||hay.includes(q);
      }).sort((a,b)=>a.code.localeCompare(b.code,'en',{numeric:true}));
    };
    const render = () => {
      const chain=state.path.map(c=>state.accounts.find(a=>a.code===c)).filter(Boolean);
      const crumbs=['<button type="button" data-path="">حساب‌های کل</button>'];
      chain.forEach((a,i)=>crumbs.push('<span>‹</span><button type="button" data-path="'+esc(chain.slice(0,i+1).map(x=>x.code).join(','))+'">'+fa(a.code)+' — '+esc(a.name)+'</button>'));
      $('[data-breadcrumb]').innerHTML=crumbs.join('');
      $('[data-breadcrumb]').querySelectorAll('[data-path]').forEach(b=>b.addEventListener('click',()=>{state.path=b.dataset.path?b.dataset.path.split(','):[];render();}));
      const list=filtered(); $('[data-count]').textContent=fa(list.length)+' حساب';
      rows.innerHTML=list.length?list.map((a,i)=>
        '<tr tabindex="'+(a.code===state.selected?'0':'-1')+'" aria-selected="'+(a.code===state.selected?'true':'false')+'" data-row="'+esc(a.code)+'" class="'+(a.code===state.selected?'selected':'')+'">'+
        '<td>'+fa(i+1)+'</td><td class="mono" dir="ltr">'+fa(a.code)+'</td><td class="name">'+esc(a.name)+'</td>'+
        '<td>'+levelText(a.level)+'</td><td>'+esc(a.groupName)+'</td><td>'+natureText(a.nature)+'</td>'+
        '<td>'+esc(a.parentName||'—')+'</td><td>'+(a.isPostable?'قابل ثبت':'غیرقابل ثبت')+'</td></tr>').join(''):
        '<tr><td colspan="8" class="adp2-empty">حسابی در این سطح پیدا نشد.</td></tr>';
      rows.querySelectorAll('[data-row]').forEach(row=>{
        row.addEventListener('click',()=>{state.selected=row.dataset.row;render();});
        row.addEventListener('dblclick',()=>{const a=getRow(row.dataset.row);if(children(a.code).length){state.path=findPath(a);state.selected=a.code;render();}else edit(a);});
        row.addEventListener('keydown',e=>{if(e.key==='Enter'){e.preventDefault();row.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));}});
      });
      $('[data-edit]').disabled=!getSelected()||state.mode!=='view';
      $('[data-delete]').disabled=!getSelected()||state.mode!=='view';
      $('[data-new]').disabled=state.mode!=='view';
      $('[data-delete-all]').disabled=state.mode!=='view'||!state.accounts.length;
    };
    const getRow = c => state.accounts.find(a=>a.code===c);
    const refresh = async resetSelection => {
      state.busy=true;say('در حال دریافت حساب‌ها و گروه‌ها…');
      try{
        const result=await Promise.all([invoke('list_accounts'),invoke('list_account_groups')]);
        state.accounts=result[0].accounts||[];state.groups=result[1].groups||[];fillGroups();
        if(resetSelection)state.selected='';
        state.path=state.path.filter((c,i)=>{const a=getRow(c);return a&&Number(a.level)===i+1&&(a.parentCode||'')===(i?state.path[i-1]:'');});
        render();say(fa(state.accounts.length)+' حساب بارگذاری شد.','ok');
      }catch(e){say(e.message||String(e),'error');}
      finally{state.busy=false;}
    };
    const parentsFor = (preferred) => {
      const n=Number(level.value);
      if(n===1){parent.innerHTML='<option value="">حساب سطح کل (بدون والد)</option>';parent.disabled=true;group.disabled=false;postable.checked=false;postable.disabled=true;if(state.mode==='new')code.value=fa(newCode(1,''));return;}
      parent.disabled=false;group.disabled=true;postable.disabled=false;
      const candidates=state.accounts.filter(a=>Number(a.level)===n-1&&a.groupCode===group.value);
      parent.innerHTML='<option value="">— انتخاب حساب بالاتر —</option>'+candidates.map(a=>'<option value="'+esc(a.code)+'">'+fa(a.code)+' — '+esc(a.name)+'</option>').join('');
      parent.value=preferred||candidates[0]?.code||'';
      if(state.mode==='new')code.value=fa(newCode(n,parent.value));
    };
    const startNew = () => {
      if(!state.groups.length){say('ابتدا یک گروه حساب تعریف کنید.','error');return;}
      const pc=state.path[state.path.length-1]||'', p=getRow(pc), n=p?Number(p.level)+1:1;
      if(n>3){say('در ساختار جدید فقط سه سطح کل، معین و تفصیلی ۱ پشتیبانی می‌شود. برای حساب‌های سطح چهارم قدیمی امکان ایجاد زیرحساب جدید وجود ندارد.','error');return;}
      state.mode='new';editor.hidden=false;form.reset();
      $('[data-editor-title]').textContent='تعریف حساب جدید';
      $('[data-editor-hint]').textContent='کد پیشنهادی مطابق کدهای موجود تولید می‌شود.';
      level.innerHTML='<option value="1">کل</option><option value="2">معین</option><option value="3">تفصیلی ۱</option>';
      level.value=String(n);fillGroups(p?p.groupCode:state.groups[0].code);group.disabled=!!p;
      name.value='';code.readOnly=false;postable.checked=n>1;errorBox.hidden=true;
      parentsFor(pc);if(pc)parent.value=pc;
      rememberEditorValues();
      $('[data-save]').textContent='ثبت حساب (F2)';render();name.focus();
    };
    const edit = account => {
      if(!account)return;
      state.selected=account.code;state.mode='edit';editor.hidden=false;fillGroups(account.groupCode);
      $('[data-editor-title]').textContent='اصلاح حساب';
      $('[data-editor-hint]').textContent='کد، سطح، گروه و حساب بالاتر در نسخه فعلی قابل تغییر نیستند.';
      code.value=fa(account.code);code.readOnly=true;name.value=account.name;
      level.innerHTML='<option value="'+account.level+'">'+levelText(account.level)+'</option>';level.value=String(account.level);
      group.value=account.groupCode;group.disabled=true;
      parent.innerHTML='<option value="'+esc(account.parentCode)+'">'+(account.parentCode?fa(account.parentCode)+' — '+esc(account.parentName):'حساب سطح کل')+'</option>';
      parent.disabled=true;nature.value=account.nature;postable.checked=!!account.isPostable;
      postable.disabled=Number(account.level)===1||children(account.code).length>0;
      rememberEditorValues();
      $('[data-save]').textContent='ذخیره تغییرات (F2)';errorBox.hidden=true;render();name.focus();
    };
    const cancel = () => {state.mode='view';editor.hidden=true;errorBox.hidden=true;render();};
    const save = async e => {
      e.preventDefault();if(state.busy||state.mode==='view')return;
      const c=nums(code.value),n=name.value.trim().replace(/\s+/g,' '),l=Number(level.value),g=group.value,p=l===1?'':parent.value;
      if(!c||!n||!g||(l>1&&!p)){errorBox.textContent='کد، نام، گروه و حساب بالاتر الزامی هستند.';errorBox.hidden=false;return;}
      const button=$('[data-save]');button.disabled=true;state.busy=true;
      try{
        if(state.mode==='edit')await invoke('update_account',{id:getSelected().id,name:n,groupCode:g,nature:nature.value,parentCode:p,isPostable:postable.checked});
        else await invoke('create_account',{code:c,name:n,level:l,groupCode:g,nature:nature.value,parentCode:p,isPostable:postable.checked});
        window.dispatchEvent(new Event('accounts-changed'));
        state.selected=c;state.mode='view';editor.hidden=true;await refresh(false);state.selected=c;render();say('اطلاعات حساب ذخیره شد.','ok');
      }catch(err){errorBox.textContent=err.message||String(err);errorBox.hidden=false;}
      finally{state.busy=false;button.disabled=false;}
    };
    const removeSelected = async () => {
      const a=getSelected();if(!a||state.busy)return;
      if(!window.confirm('حساب «'+fa(a.code)+' — '+a.name+'» حذف شود؟ حساب دارای زیرحساب یا گردش سند قابل حذف نیست.'))return;
      state.busy=true;
      try{await invoke('delete_account',{id:a.id});window.dispatchEvent(new Event('accounts-changed'));state.selected='';await refresh(false);say('حساب حذف شد.','ok');}
      catch(e){say(e.message||String(e),'error');}finally{state.busy=false;}
    };
    const removeAll = async () => {
      if(!state.accounts.length||state.busy)return;
      if(!window.confirm('تلاش برای حذف همه '+fa(state.accounts.length)+' حساب انجام شود؟ حساب‌های دارای زیرحساب یا سابقه سند باقی می‌مانند.'))return;
      state.busy=true;let done=0;const blocked=[];
      try{
        const ordered=[...state.accounts].sort((a,b)=>Number(b.level)-Number(a.level));
        for(const a of ordered){try{await invoke('delete_account',{id:a.id});done++;}catch(e){blocked.push(fa(a.code)+' — '+a.name+': '+(e.message||String(e)));}}
        if(done)window.dispatchEvent(new Event('accounts-changed'));
        state.selected='';await refresh(false);say(fa(done)+' حساب حذف شد؛ '+fa(blocked.length)+' حساب باقی ماند.',blocked.length?'error':'ok');
        if(blocked.length)window.alert('حساب‌های حذف‌نشده:\n'+blocked.slice(0,25).join('\n')+(blocked.length>25?'\n…':''));
      }finally{state.busy=false;}
    };
    const close = () => {
      if(state.mode!=='view'&&initialEditorValues!==editorValues()
        &&!window.confirm('تغییرات ذخیره‌نشدهٔ فرم حساب کنار گذاشته شود؟'))return;
      overlay.remove();document.removeEventListener('keydown',keys,true);
    };
    const keys = e => {
      if(e.key==='Escape'){e.preventDefault();close();return;}
      if(e.key==='F2'&&state.mode!=='view'){e.preventDefault();form.requestSubmit();return;}
      if(e.key==='Tab'){
        const focusable=Array.from(overlay.querySelectorAll('button:not(:disabled),input:not(:disabled),select:not(:disabled),[tabindex]:not([tabindex="-1"])'))
          .filter(el=>!el.hidden&&el.getClientRects().length>0);
        if(!focusable.length)return;
        const first=focusable[0],last=focusable[focusable.length-1];
        if(e.shiftKey&&document.activeElement===first){e.preventDefault();last.focus();}
        else if(!e.shiftKey&&document.activeElement===last){e.preventDefault();first.focus();}
      }
    };
    $('[data-close]').addEventListener('click',close);
    $('[data-new]').addEventListener('click',startNew);
    $('[data-edit]').addEventListener('click',()=>edit(getSelected()));
    $('[data-delete]').addEventListener('click',removeSelected);
    $('[data-delete-all]').addEventListener('click',removeAll);
    $('[data-refresh]').addEventListener('click',()=>refresh(false));
    $('[data-reset]').addEventListener('click',()=>{state.search='';state.group='';state.level='';state.path=[];$('[data-search]').value='';$('[data-group]').value='';$('[data-level]').value='';render();});
    $('[data-search]').addEventListener('input',e=>{state.search=e.target.value;render();});
    $('[data-group]').addEventListener('change',e=>{state.group=e.target.value;render();});
    $('[data-level]').addEventListener('change',e=>{state.level=e.target.value;render();});
    level.addEventListener('change',()=>parentsFor(''));
    group.addEventListener('change',()=>parentsFor(''));
    parent.addEventListener('change',()=>{if(state.mode==='new')code.value=fa(newCode(Number(level.value),parent.value));});
    code.addEventListener('input',()=>{if(state.mode==='new')code.value=fa(nums(code.value));});
    form.addEventListener('submit',save);$('[data-cancel]').addEventListener('click',cancel);
    $('[data-export]').addEventListener('click',async()=>{try{const path=await invoke('export_account_excel');if(path)say('فایل اکسل ذخیره شد: '+path,'ok');}catch(e){say(e.message||String(e),'error');}});
    $('[data-import]').addEventListener('click',async()=>{if(!window.confirm('حساب‌ها از فایل اکسل مطابق قالب WebApp وارد شوند؟'))return;try{const result=await invoke('import_account_excel');if(!result)return;window.dispatchEvent(new Event('accounts-changed'));await refresh(true);const msg=fa(result.createdCount)+' حساب جدید و '+fa(result.skippedCount)+' حساب موجود';say(msg+(result.errors.length?'؛ '+fa(result.errors.length)+' خطا':''),result.errors.length?'error':'ok');if(result.errors.length)window.alert('موارد ناموفق:\n'+result.errors.slice(0,30).join('\n'));}catch(e){say(e.message||String(e),'error');}});
    overlay.addEventListener('mousedown',e=>{if(e.target===overlay&&state.mode==='view')close();});
    document.addEventListener('keydown',keys,true);document.body.appendChild(overlay);
    refresh(true);setTimeout(()=>$('[data-search]').focus(),30);
  }
  window.openAccountDefinitionPopup = openAccountDefinitionPopup;
})();