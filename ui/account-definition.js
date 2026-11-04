/* WebApp-inspired account-definition popup for the Rust/Tauri client. */
(() => {
  'use strict';
  const tauriCore = window.__TAURI__?.core || null;
  const invoke = typeof tauriCore?.invoke === 'function'
    ? tauriCore.invoke.bind(tauriCore)
    : null;
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
    window.dispatchEvent(new Event('account-definition-opened'));
    invoke('open_account_definition_window').catch(error => window.alert(error.message || String(error)));
  }

  function renderAccountDefinitionForm() {
    const state = { accounts:[], groups:[], selected:'', path:[], mode:'view', busy:false, search:'', group:'', level:'', active:'' };
    const overlay = document.createElement('main');
    overlay.className = 'adp2-window';
    overlay.innerHTML =
      '<div class="adp2-content">' +
        '<nav class="adp2-breadcrumb" data-breadcrumb aria-label="مسیر حساب‌ها"></nav>' +
        '<div class="adp2-filters">' +
          '<label>جست‌وجو<input type="search" data-search placeholder="کد، نام یا حساب بالاتر"></label>' +
          '<label>گروه حساب<select data-group><option value="">همه گروه‌ها</option></select></label>' +
          '<label>سطح<select data-level><option value="">همه سطوح</option><option value="1">کل</option><option value="2">معین</option><option value="3">تفصیلی ۱</option></select></label>' +
          '<label>وضعیت<select data-active-filter><option value="">همه</option><option value="1">فعال</option><option value="0">غیرفعال</option></select></label>' +
          '<button type="button" class="adp2-button" data-reset>پاک‌کردن فیلتر</button><button type="button" class="adp2-button" data-refresh>↻ بارگذاری مجدد</button>' +
        '</div>' +
        '<div class="adp2-grid-wrap"><table class="adp2-grid"><thead><tr><th>ردیف</th><th>کد حساب</th><th>نام حساب</th><th>سطح</th><th>گروه حساب</th><th>ماهیت</th><th>حساب بالاتر</th><th>ثبت سند</th><th>وضعیت</th></tr></thead><tbody data-rows><tr><td colspan="9">در حال بارگذاری…</td></tr></tbody></table></div>' +
        '<section class="adp2-editor" data-editor hidden><div class="adp2-editor-head"><b data-editor-title>مشخصات حساب</b><span data-editor-hint></span></div>' +
          '<form data-form class="adp2-form" autocomplete="off">' +
            '<label>کد حساب<input data-code dir="ltr" inputmode="numeric" maxlength="50" required aria-describedby="adpFormError"></label>' +
            '<label class="adp2-wide">نام حساب<input data-name maxlength="255" required></label>' +
            '<label>سطح حساب<select data-edit-level><option value="1">کل</option><option value="2">معین</option><option value="3">تفصیلی ۱</option></select></label>' +
            '<label>گروه حساب<select data-edit-group required></select></label>' +
            '<label>حساب بالاتر<select data-parent></select></label>' +
            '<label>ماهیت<select data-nature><option value="debtor">بدهکار</option><option value="creditor">بستانکار</option><option value="dual">دوگانه</option></select></label>' +
            '<label class="adp2-check"><input type="checkbox" data-postable> قابل ثبت در سند</label>' +
            '<label class="adp2-check"><input type="checkbox" data-active-check> حساب فعال</label>' +
            '<div class="adp2-form-error" id="adpFormError" data-error role="alert" aria-live="assertive" hidden></div><div class="adp2-form-actions">' +
              '<button type="submit" class="adp2-button primary" data-save>ذخیره (F2)</button><button type="button" class="adp2-button" data-cancel>انصراف</button>' +
            '</div></form></section>' +
      '</div><footer class="adp2-toolbar">' +
        '<button type="button" class="adp2-button" data-new>＋ حساب جدید</button><button type="button" class="adp2-button" data-edit>✎ اصلاح</button>' +
        '<button type="button" class="adp2-button danger" data-delete>حذف حساب</button><button type="button" class="adp2-button danger" data-delete-all>حذف همه</button>' +
        '<span class="adp2-spacer"></span><button type="button" class="adp2-button" data-export>خروجی اکسل</button><button type="button" class="adp2-button" data-import>ورودی اکسل</button><span class="adp2-count" data-count></span>' +
      '</footer><div class="adp2-status" data-status role="status"></div>';
    document.body.appendChild(overlay);

    const $ = s => overlay.querySelector(s);
    const rows = $('[data-rows]'), editor = $('[data-editor]'), form = $('[data-form]');
    const code = $('[data-code]'), name = $('[data-name]'), level = $('[data-edit-level]');
    const group = $('[data-edit-group]'), parent = $('[data-parent]'), nature = $('[data-nature]');
    const postable = $('[data-postable]'), errorBox = $('[data-error]'), status = $('[data-status]');
    const activeCheck = $('[data-active-check]');
    let initialEditorValues = null;
    const editorValues = () => JSON.stringify([
      code.value, name.value, level.value, group.value, parent.value, nature.value, postable.checked, activeCheck.checked,
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
      const maxWidth=50-prefix.length;
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
    const setSuggestedCode = (n,p) => {
      const suggested=newCode(n,p);
      code.value=fa(suggested);
      if(!suggested)say('کد آزادی در این سطح موجود نیست؛ کد را دستی وارد کنید.','error');
      return suggested;
    };
    const fillGroups = selected => {
      const options=state.groups.map(g=>'<option value="'+esc(g.code)+'">'+fa(g.code)+' — '+esc(g.name)+' ('+esc(g.type)+')</option>').join('');
      $('[data-group]').innerHTML='<option value="">همه گروه‌ها</option>'+options;
      group.innerHTML=options; if(selected) group.value=selected;
      $('[data-group]').value=state.group;
    };
    const filtered = () => {
      const q=searchText(state.search), hasFilter=!!(q||state.group||state.level||state.active);
      const current=state.path[state.path.length-1]||'';
      return state.accounts.filter(a=>{
        if(!hasFilter && (current?a.parentCode!==current:Number(a.level)!==1)) return false;
        if(state.group && a.groupCode!==state.group) return false;
        if(state.level && Number(a.level)!==Number(state.level)) return false;
        if(state.active !== '' && Boolean(a.isActive)!==(state.active==='1')) return false;
        const hay=searchText([a.code,a.name,a.parentName,a.groupName].join(' '));
        return !q||hay.includes(q);
      }).sort((a,b)=>a.code.localeCompare(b.code,'en',{numeric:true}));
    };
    const render = () => {
      const chain=state.path.map(c=>state.accounts.find(a=>a.code===c)).filter(Boolean);
      const crumbs=['<button type="button" data-path="">حساب‌های کل</button>'];
      chain.forEach((a,i)=>crumbs.push('<span>‹</span><button type="button" data-path="'+esc(chain.slice(0,i+1).map(x=>x.code).join(','))+'">'+fa(a.code)+' — '+esc(a.name)+'</button>'));
      $('[data-breadcrumb]').innerHTML=crumbs.join('');
      $('[data-breadcrumb]').querySelectorAll('[data-path]').forEach(b=>{
        b.disabled=state.busy;
        b.addEventListener('click',()=>{
          if(state.busy)return;
          state.path=b.dataset.path?b.dataset.path.split(','):[];render();
        });
      });
      const list=filtered();
      if(!list.some(a=>a.code===state.selected))state.selected=list[0]?.code||'';
      $('[data-count]').textContent=fa(list.length)+' حساب';
      rows.innerHTML=list.length?list.map((a,i)=>
        '<tr tabindex="'+(a.code===state.selected?'0':'-1')+'" aria-selected="'+(a.code===state.selected?'true':'false')+'" data-row="'+esc(a.code)+'" class="'+(a.code===state.selected?'selected ':'')+(a.isActive?'':'inactive')+'">'+
        '<td>'+fa(i+1)+'</td><td class="mono" dir="ltr">'+fa(a.code)+'</td><td class="name">'+esc(a.name)+'</td>'+
        '<td>'+levelText(a.level)+'</td><td>'+esc(a.groupName)+'</td><td>'+natureText(a.nature)+'</td>'+
        '<td>'+esc(a.parentName||'—')+'</td><td>'+(a.isPostable?'قابل ثبت':'غیرقابل ثبت')+'</td>'+
        '<td>'+(a.isActive?'فعال':'غیرفعال')+'</td></tr>').join(''):
        '<tr><td colspan="9" class="adp2-empty">حسابی در این سطح پیدا نشد.</td></tr>';
      rows.querySelectorAll('[data-row]').forEach(row=>{
        row.addEventListener('click',()=>{
          if(state.busy)return;
          state.selected=row.dataset.row;render();
          rows.querySelectorAll('[data-row]').forEach(item=>{if(item.dataset.row===state.selected)item.focus();});
        });
        row.addEventListener('dblclick',()=>{
          if(state.busy)return;
          const a=getRow(row.dataset.row);
          if(children(a.code).length){state.path=findPath(a);state.selected=a.code;render();}else edit(a);
        });
        row.addEventListener('keydown',e=>{
          if(state.busy)return;
          if(e.key==='Enter'){e.preventDefault();row.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));return;}
          if(e.key!=='ArrowDown'&&e.key!=='ArrowUp')return;
          e.preventDefault();
          const visibleRows=Array.from(rows.querySelectorAll('[data-row]'));
          const index=visibleRows.indexOf(row), direction=e.key==='ArrowDown'?1:-1;
          const target=visibleRows[Math.max(0,Math.min(visibleRows.length-1,index+direction))];
          if(!target)return;
          state.selected=target.dataset.row;render();
          rows.querySelectorAll('[data-row]').forEach(item=>{if(item.dataset.row===state.selected)item.focus();});
        });
      });
      const busy=state.busy;
      for(const control of overlay.querySelectorAll('[data-search],[data-group],[data-level],[data-active-filter],[data-reset]'))control.disabled=busy;
      $('[data-edit]').disabled=!getSelected()||state.mode!=='view'||busy;
      $('[data-delete]').disabled=!getSelected()||state.mode!=='view'||busy;
      $('[data-new]').disabled=state.mode!=='view'||busy;
      $('[data-delete-all]').disabled=state.mode!=='view'||busy||!state.accounts.length;
      $('[data-refresh]').disabled=busy;
      $('[data-export]').disabled=busy;
      $('[data-import]').disabled=busy;
      $('[data-save]').disabled=busy;
      $('[data-cancel]').disabled=busy;
    };
    const getRow = c => state.accounts.find(a=>a.code===c);
    const refresh = async (resetSelection, keepBusy = false) => {
      state.busy=true;render();say('در حال دریافت حساب‌ها و گروه‌ها…');
      try{
        const result=await Promise.all([invoke('list_accounts'),invoke('list_account_groups')]);
        state.accounts=result[0].accounts||[];state.groups=result[1].groups||[];fillGroups();
        if(resetSelection)state.selected='';
        state.path=state.path.filter((c,i)=>{const a=getRow(c);return a&&Number(a.level)===i+1&&(a.parentCode||'')===(i?state.path[i-1]:'');});
        render();say(fa(state.accounts.length)+' حساب بارگذاری شد.','ok');
      }catch(e){say(e.message||String(e),'error');}
      finally{if(!keepBusy)state.busy=false;render();}
    };
    const parentsFor = (preferred, allowGroupChange = false) => {
      const n=Number(level.value);
      if(n===1){parent.innerHTML='<option value="">حساب سطح کل (بدون والد)</option>';parent.disabled=true;group.disabled=false;postable.checked=false;postable.disabled=true;if(state.mode==='new')setSuggestedCode(1,'');return;}
      parent.disabled=false;group.disabled=!allowGroupChange;postable.disabled=false;
      const candidates=state.accounts.filter(a=>Number(a.level)===n-1&&a.groupCode===group.value&&(a.isActive!==false||(state.mode==='edit'&&a.code===preferred)));
      parent.innerHTML='<option value="">— انتخاب حساب بالاتر —</option>'+candidates.map(a=>'<option value="'+esc(a.code)+'">'+fa(a.code)+' — '+esc(a.name)+(a.isActive===false?' (غیرفعال)':'')+'</option>').join('');
      const preferredAvailable=candidates.some(a=>a.code===preferred);
      parent.value=preferredAvailable?preferred:(preferred&&state.mode==='new'?'':(candidates[0]?.code||''));
      if(state.mode==='new')setSuggestedCode(n,parent.value);
    };
    const startNew = () => {
      if(state.busy||state.mode!=='view')return;
      if(!state.groups.length){say('ابتدا یک گروه حساب تعریف کنید.','error');return;}
      const pc=state.path[state.path.length-1]||'', p=getRow(pc), n=p?Number(p.level)+1:1;
      if(n>3){say('پس از گروه حساب، سه سطح قابل تعریف است: کل، معین و تفصیلی ۱. حساب‌های تفصیلی ۲ قدیمی فقط برای مشاهده و سازگاری با اسناد قبلی نگهداری می‌شوند.','error');return;}
      state.mode='new';editor.hidden=false;form.reset();
      $('[data-editor-title]').textContent='تعریف حساب جدید';
      $('[data-editor-hint]').textContent='کد پیشنهادی مطابق کدهای موجود تولید می‌شود.';
      level.innerHTML='<option value="1">کل</option><option value="2">معین</option><option value="3">تفصیلی ۱</option>';
      level.value=String(n);fillGroups(p?p.groupCode:state.groups[0].code);group.disabled=!!p;
      name.value='';code.readOnly=false;postable.checked=n>1;activeCheck.checked=true;errorBox.hidden=true;code.removeAttribute('aria-invalid');
      parentsFor(pc);if(pc)parent.value=pc;
      rememberEditorValues();
      $('[data-save]').textContent='ثبت حساب (F2)';render();name.focus();
    };
    const edit = account => {
      if(state.busy||state.mode!=='view'||!account)return;
      state.selected=account.code;state.mode='edit';editor.hidden=false;fillGroups(account.groupCode);
      $('[data-editor-title]').textContent='اصلاح حساب';
      $('[data-editor-hint]').textContent='گروه و حساب بالاتر فقط وقتی قابل تغییرند که حساب در سندی استفاده نشده باشد.';
      code.value=fa(account.code);code.readOnly=true;name.value=account.name;
      level.innerHTML='<option value="'+account.level+'">'+levelText(account.level)+'</option>';level.value=String(account.level);
      group.value=account.groupCode;
      parentsFor(account.parentCode,true);
      nature.value=account.nature;postable.checked=!!account.isPostable;activeCheck.checked=account.isActive!==false;
      postable.disabled=Number(account.level)===1||children(account.code).length>0;
      rememberEditorValues();
      $('[data-save]').textContent='ذخیره تغییرات (F2)';errorBox.hidden=true;code.removeAttribute('aria-invalid');render();name.focus();
    };
    const cancel = () => {state.mode='view';editor.hidden=true;errorBox.hidden=true;code.removeAttribute('aria-invalid');render();};
    const save = async e => {
      e.preventDefault();if(state.busy||state.mode==='view')return;
      const c=nums(code.value),n=name.value.trim().replace(/\s+/g,' '),l=Number(level.value),g=group.value,p=l===1?'':parent.value;
      if(!c||!n||!g||(l>1&&!p)){
        errorBox.textContent='کد، نام، گروه و حساب بالاتر الزامی هستند.';errorBox.hidden=false;
        code.setAttribute('aria-invalid', String(!c));
        if(!c)code.focus();
        return;
      }
      code.removeAttribute('aria-invalid');
      const button=$('[data-save]');button.disabled=true;state.busy=true;
      try{
        if(state.mode==='edit')await invoke('update_account',{id:getSelected().id,name:n,groupCode:g,nature:nature.value,parentCode:p,isPostable:postable.checked,isActive:activeCheck.checked});
        else await invoke('create_account',{code:c,name:n,level:l,groupCode:g,nature:nature.value,parentCode:p,isPostable:postable.checked,isActive:activeCheck.checked});
        window.dispatchEvent(new Event('accounts-changed'));
        state.selected=c;state.mode='view';editor.hidden=true;await refresh(false,true);state.selected=c;render();say('اطلاعات حساب ذخیره شد.','ok');
      }catch(err){errorBox.textContent=err.message||String(err);errorBox.hidden=false;}
      finally{state.busy=false;button.disabled=false;render();}
    };
    const removeSelected = async () => {
      const a=getSelected();if(!a||state.busy)return;
      if(!window.confirm('حساب «'+fa(a.code)+' — '+a.name+'» حذف شود؟ حساب دارای زیرحساب یا گردش سند قابل حذف نیست.'))return;
      state.busy=true;render();
      try{await invoke('delete_account',{id:a.id});window.dispatchEvent(new Event('accounts-changed'));state.selected='';await refresh(false,true);say('حساب حذف شد.','ok');}
      catch(e){say(e.message||String(e),'error');}finally{state.busy=false;render();}
    };
    const removeAll = async () => {
      if(!state.accounts.length||state.busy)return;
      if(!window.confirm('تلاش برای حذف همه '+fa(state.accounts.length)+' حساب انجام شود؟ حساب‌های دارای زیرحساب یا سابقه سند باقی می‌مانند.'))return;
      if(window.prompt('برای تأیید حذف گروهی، کلمه «حذف» را وارد کنید.')!=='حذف')return;
      state.busy=true;render();let done=0;const blocked=[];
      try{
        const ordered=[...state.accounts].sort((a,b)=>Number(b.level)-Number(a.level));
        const result=await invoke('delete_accounts_bulk',{ids:ordered.map(a=>a.id)});
        done=Number(result?.deletedCount)||0;
        for(const item of result?.blocked||[]){
          blocked.push(fa(item.code)+' — '+item.name+': '+(item.error||''));
        }
        if(done)window.dispatchEvent(new Event('accounts-changed'));
        state.selected='';await refresh(false,true);say(fa(done)+' حساب حذف شد؛ '+fa(blocked.length)+' حساب باقی ماند.',blocked.length?'error':'ok');
        if(blocked.length)window.alert('حساب‌های حذف‌نشده:\n'+blocked.slice(0,25).join('\n')+(blocked.length>25?'\n…':''));
      }catch(e){say(e.message||String(e),'error');}finally{state.busy=false;render();}
    };
    const currentWindow = window.__TAURI__?.window?.getCurrentWindow?.();
    const hasUnsavedChanges = () => state.mode !== 'view' && initialEditorValues !== editorValues();
    let closeApproved = false;
    const close = () => {
      if(hasUnsavedChanges()&&!window.confirm('تغییرات ذخیره‌نشدهٔ فرم حساب کنار گذاشته شود؟'))return;
      document.removeEventListener('keydown',keys,true);
      closeApproved = true;
      currentWindow?.close();
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
    currentWindow?.onCloseRequested?.((event) => {
      if (closeApproved) return;
      if (hasUnsavedChanges() && !window.confirm('تغییرات ذخیره‌نشدهٔ فرم حساب کنار گذاشته شود؟')) {
        event.preventDefault();
        return;
      }
      closeApproved = true;
    });
    $('[data-new]').addEventListener('click',startNew);
    $('[data-edit]').addEventListener('click',()=>edit(getSelected()));
    $('[data-delete]').addEventListener('click',removeSelected);
    $('[data-delete-all]').addEventListener('click',removeAll);
    $('[data-refresh]').addEventListener('click',()=>refresh(false));
    $('[data-reset]').addEventListener('click',()=>{state.search='';state.group='';state.level='';state.active='';state.path=[];$('[data-search]').value='';$('[data-group]').value='';$('[data-level]').value='';$('[data-active-filter]').value='';render();});
    $('[data-search]').addEventListener('input',e=>{state.search=e.target.value;render();});
    $('[data-group]').addEventListener('change',e=>{state.group=e.target.value;render();});
    $('[data-level]').addEventListener('change',e=>{state.level=e.target.value;render();});
    $('[data-active-filter]').addEventListener('change',e=>{state.active=e.target.value;render();});
    level.addEventListener('change',()=>parentsFor(''));
    group.addEventListener('change',()=>parentsFor('',state.mode==='edit'));
    parent.addEventListener('change',()=>{if(state.mode==='new')setSuggestedCode(Number(level.value),parent.value);});
    code.addEventListener('input',()=>{if(state.mode==='new')code.value=fa(nums(code.value));});
    form.addEventListener('submit',save);$('[data-cancel]').addEventListener('click',cancel);
    $('[data-export]').addEventListener('click',async()=>{
      if(state.busy)return;
      state.busy=true;render();
      try{
        const result=await invoke('export_account_excel');
        if(result){
          say('فایل اکسل ذخیره شد: '+result.path,'ok');
          if(result.skippedCodes?.length){
            window.alert('حساب‌های سطح چهارم در قالب فعلی صادر نشدند:\n'+result.skippedCodes.map(fa).join('، '));
          }
        }
      }
      catch(e){say(e.message||String(e),'error');}
      finally{state.busy=false;render();}
    });
    $('[data-import]').addEventListener('click',async()=>{
      if(state.busy||!window.confirm('حساب‌ها از فایل اکسل مطابق قالب WebApp وارد شوند؟'))return;
      state.busy=true;render();
      try{
        const result=await invoke('import_account_excel');
        if(!result)return;
        window.dispatchEvent(new Event('accounts-changed'));
        await refresh(true,true);
        const msg=fa(result.createdCount)+' حساب جدید و '+fa(result.skippedCount)+' حساب موجود';
        say(msg+(result.errors.length?'؛ '+fa(result.errors.length)+' خطا':''),result.errors.length?'error':'ok');
        if(result.errors.length){
          const shown=result.errors.slice(0,30);
          const rest=result.errors.length-shown.length;
          window.alert('موارد ناموفق'+(rest?' (نمایش '+fa(shown.length)+' مورد از '+fa(result.errors.length)+')':'')+':\n'+shown.join('\n'));
        }
      }catch(e){say(e.message||String(e),'error');}
      finally{state.busy=false;render();}
    });
    document.addEventListener('keydown',keys,true);document.body.appendChild(overlay);
    refresh(true);setTimeout(()=>$('[data-search]').focus(),30);
  }
  window.openAccountDefinitionPopup = openAccountDefinitionPopup;
  window.openAccountDefinitionForm = renderAccountDefinitionForm;
  if (document.body.dataset.accountDefinition === 'true') renderAccountDefinitionForm();
})();