'use strict';

function applyVoucherGridSearch() {
  const input = $('#vfGridSearch');
  const count = $('#vfGridSearchCount');
  const query = normalizeSearch(input ? input.value : '');
  let visible = 0;
  let filled = 0;
  for (const row of gridRows()) {
    if (!rowHasValue(row)) { row.hidden = !!query; continue; }
    filled += 1;
    const code = digitsOnly(row.querySelector('.vf-code-input')?.value || '');
    const account = accountForCode(code);
    const haystack = normalizeSearch([
      code, account ? account.name : '',
      row.querySelector('.vf-detail1-input')?.value || '',
      row.querySelector('.vf-desc-input')?.value || '',
      row.querySelector('.vf-debit-input')?.value || '',
      row.querySelector('.vf-credit-input')?.value || '',
    ].join(' '));
    const activeEditing = row === state.activeRow &&
      document.activeElement?.closest?.('#voucherRows tr') === row;
    const match = !query || haystack.includes(query) || activeEditing;
    row.hidden = !match;
    if (match) visible += 1;
  }
  if (count) count.textContent = query
    ? vfFa(visible) + ' از ' + vfFa(filled) + ' ردیف'
    : vfFa(filled) + ' ردیف';
}

function focusedVoucherRow() {
  const active = document.activeElement;
  const row = active && active.closest ? active.closest('#voucherRows tr') : null;
  return row || (state.activeRow && state.activeRow.isConnected ? state.activeRow : null);
}
function writeClipboardText(text) {
  if (navigator.clipboard && navigator.clipboard.writeText) return navigator.clipboard.writeText(text);
  return new Promise((resolve, reject) => {
    const area = document.createElement('textarea');
    area.value = text; area.style.position = 'fixed'; area.style.opacity = '0';
    document.body.append(area); area.select();
    const ok = document.execCommand('copy'); area.remove();
    if (ok) resolve(); else reject(new Error('دسترسی به کلیپ‌بورد ممکن نیست.'));
  });
}
async function copyVoucherRow() {
  const row = focusedVoucherRow();
  if (!row || !rowHasValue(row)) { toast('ابتدا یک ردیف دارای اطلاعات را انتخاب کنید.', 'info'); return; }
  const code = digitsOnly(row.querySelector('.vf-code-input')?.value || '');
  const detail1 = digitsOnly(row.querySelector('.vf-detail1-input')?.value || '');
  const description = row.querySelector('.vf-desc-input')?.value || '';
  const debit = digitsOnly(row.querySelector('.vf-debit-input')?.value || '');
  const credit = digitsOnly(row.querySelector('.vf-credit-input')?.value || '');
  const rowNumber = gridRows().filter(rowHasValue).indexOf(row) + 1;
  const line = [rowNumber, code, detail1, description, debit, credit].join('\t');
  try { await writeClipboardText(line); state.activeRow = row; toast('ردیف جاری در کلیپ‌بورد کپی شد.', 'ok'); }
  catch (error) { toast(errorText(error), 'error'); }
}
function parseVoucherClipboardRows(raw) {
  const lines = String(raw || '').split(/\r?\n/).filter((line) => line.trim());
  if (!lines.length) return [];
  const isHeader = (cells) => {
    const words = cells.map(normalizeSearch).join(' ');
    const hasCode = words.includes('کد') || words.includes('code') || words.includes('account');
    const hasDebit = words.includes('بدهکار') || words.includes('debit');
    const hasCredit = words.includes('بستانکار') || words.includes('credit');
    return hasCode && hasDebit && hasCredit;
  };
  const start = isHeader(lines[0].split('\t')) ? 1 : 0;
  const parsed = [];
  for (const line of lines.slice(start)) {
    const cells = line.split('\t').map((cell) => cell.trim());
    let code = '', detail1 = '', description = '', debitText = '', creditText = '';
    if (cells.length >= 6) {
      code = cells[1] || '';
      detail1 = cells[2] || '';
      description = cells[3] || '';
      debitText = cells[4] || '';
      creditText = cells[5] || '';
    } else if (cells.length >= 5) {
      code = cells[1] && accountForCode(cells[1]) ? cells[1] : (cells[0] || '');
      description = cells[2] || ''; debitText = cells[3] || ''; creditText = cells[4] || '';
    } else {
      code = cells[0] || ''; description = cells[1] || '';
      debitText = cells[2] || ''; creditText = cells[3] || '';
    }
    const accountCode = digitsOnly(code);
    const detail1Code = digitsOnly(detail1);
    const descriptionLength = [...description].length;
    if (descriptionLength > MAX_LINE_DESCRIPTION) {
      throw new Error(`شرح ردیف کلیپ‌بورد بیش از ${vfFa(MAX_LINE_DESCRIPTION)} نویسه است (${vfFa(descriptionLength)} نویسه).`);
    }
    const parseClipboardAmount = (text) => {
      const normalized = toAsciiDigits(text).replace(/[,٬\s\u200c\u00a0]/g, '');
      if (/[-−]/.test(normalized)) throw new Error('مبلغ منفی مجاز نیست؛ از ستون مقابل استفاده کنید.');
      if (/[./٫]/.test(normalized)) throw new Error('مبلغ باید عدد صحیح ریالی باشد؛ اعشار مجاز نیست.');
      if (!normalized) return 0;
      if (!/^\d+$/.test(normalized)) throw new Error(`مبلغ «${text}» معتبر نیست.`);
      const amount = Number(normalized);
      if (!Number.isSafeInteger(amount) || amount > Number(MAX_AMOUNT))
        throw new Error('یکی از مبالغ کلیپ‌بورد خارج از محدوده مجاز است.');
      return amount;
    };
    const debit = parseClipboardAmount(debitText);
    const credit = parseClipboardAmount(creditText);
    if (!accountCode && !detail1Code && !description && !debit && !credit) continue;
    if (!Number.isSafeInteger(debit) || !Number.isSafeInteger(credit) ||
        debit > Number(MAX_AMOUNT) || credit > Number(MAX_AMOUNT))
      throw new Error('یکی از مبالغ کلیپ‌بورد خارج از محدوده مجاز است.');
    if (debit > 0 && credit > 0)
      throw new Error('در ردیف چسبانده‌شده، هم‌زمان بدهکار و بستانکار پر شده است.');
    parsed.push({ accountCode, detail1Code, description, debit, credit });
  }
  return parsed;
}
async function pasteVoucherRows() {
  try {
    if (!navigator.clipboard || !navigator.clipboard.readText)
      throw new Error('خواندن کلیپ‌بورد در این پنجره مجاز نیست.');
    const parsed = parseVoucherClipboardRows(await navigator.clipboard.readText());
    if (!parsed.length) { toast('ردیف قابل استفاده‌ای در کلیپ‌بورد پیدا نشد.', 'info'); return; }
    const tbody = $('#voucherRows'); if (!tbody) return;
    const existingCount = gridRows().filter(rowHasValue).length;
    const availableCount = Math.max(0, MAX_VOUCHER_LINES - existingCount);
    if (parsed.length > availableCount) {
      toast(`ظرفیت باقی‌مانده ${vfFa(availableCount)} ردیف است، اما ${vfFa(parsed.length)} ردیف درخواست شده؛ هیچ ردیفی وارد نشد.`, 'warning');
      return;
    }
    const filter = $('#vfGridSearch'); if (filter) filter.value = '';
    let anchor = focusedVoucherRow();
    if (anchor && !rowHasValue(anchor)) anchor = anchor.previousElementSibling;
    if (!anchor || anchor.parentElement !== tbody) {
      const filled = gridRows().filter(rowHasValue);
      anchor = filled.length ? filled[filled.length - 1] : null;
    }
    let inserted = 0;
    for (const line of parsed) {
      if (anchor) anchor.insertAdjacentHTML('afterend', voucherRowTemplate());
      else tbody.insertAdjacentHTML('afterbegin', voucherRowTemplate());
      const row = anchor ? anchor.nextElementSibling : tbody.firstElementChild;
      if (!row) break;
      row.querySelector('.vf-code-input').value = line.accountCode;
      row.querySelector('.vf-detail1-input').value = line.detail1Code || '';
      row.querySelector('.vf-desc-input').value = line.description;
      row.querySelector('.vf-debit-input').value = line.debit ? formatAmount(line.debit) : '';
      row.querySelector('.vf-credit-input').value = line.credit ? formatAmount(line.credit) : '';
      updateRowAccount(row, true); anchor = row; inserted += 1;
    }
    if (!inserted) { toast('ظرفیت مجاز ردیف‌های سند تکمیل است.', 'warning'); return; }
    state.activeRow = anchor; ensureTrailingBlank(); syncVoucherRowCount();
    refreshFormState(); markDirty();
    toast(vfFa(inserted) + ' ردیف از کلیپ‌بورد چسبانده شد.', 'ok');
    anchor.querySelector('.vf-code-input')?.focus();
  } catch (error) { toast(errorText(error), 'error'); }
}
function transferModalOpen() {
  const modal = $('#vfTransferModal'); return !!modal && !modal.hidden;
}
function excelPreviewOpen() {
  const modal = $('#vfExcelPreviewModal'); return !!modal && !modal.hidden;
}
function renderVoucherTransferSummary() {
  const source = state.transfer.source, summary = $('#vfTransferSummary');
  if (!source || !summary) return;
  const chosen = source.lines.filter((line) => state.transfer.selectedIds.has(line.id));
  const debit = chosen.reduce((sum, line) => sum + BigInt(line.debit || 0), 0n);
  const credit = chosen.reduce((sum, line) => sum + BigInt(line.credit || 0), 0n);
  const difference = debit > credit ? debit - credit : credit - debit;
  summary.textContent = vfFa(chosen.length) + ' ردیف | بدهکار ' +
    vfFa(formatAmount(debit)) + ' | بستانکار ' + vfFa(formatAmount(credit)) +
    ' | مغایرت ' + vfFa(formatAmount(difference));
}
function renderVoucherTransferRows() {
  const tbody = $('#vfTransferRows'), source = state.transfer.source;
  if (!tbody || !source) return;
  tbody.textContent = '';
  for (const line of source.lines) {
    const row = document.createElement('tr');
    const pickCell = document.createElement('td'); pickCell.className = 'vf-center';
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox'; checkbox.className = 'vf-transfer-check';
    checkbox.dataset.lineId = String(line.id);
    checkbox.checked = state.transfer.selectedIds.has(line.id);
    checkbox.setAttribute('aria-label', 'انتخاب ردیف ' + vfFa(line.rowNo));
    pickCell.append(checkbox); row.append(pickCell);
    const values = [vfFa(line.rowNo), vfFa(line.accountCode), line.accountName || '',
      line.description || '', vfFa(formatAmount(line.debit || 0)), vfFa(formatAmount(line.credit || 0))];
    values.forEach((value, index) => {
      const cell = document.createElement('td'); cell.textContent = value;
      if (index >= 4) cell.className = 'vf-money'; row.append(cell);
    });
    if (checkbox.checked) row.classList.add('selected');
    checkbox.addEventListener('change', () => {
      if (checkbox.checked) state.transfer.selectedIds.add(line.id);
      else state.transfer.selectedIds.delete(line.id);
      row.classList.toggle('selected', checkbox.checked); renderVoucherTransferSummary();
    });
    row.addEventListener('click', (event) => {
      if (event.target === checkbox) return;
      checkbox.checked = !checkbox.checked;
      checkbox.dispatchEvent(new Event('change', { bubbles: true }));
    });
    tbody.append(row);
  }
  renderVoucherTransferSummary();
}
async function openVoucherTransfer(mode) {
  if (state.busy || !invoke || (mode !== 'copy' && mode !== 'move')) return;
  let source = state.voucher;
  if (!source) { toast('برای کپی یا انتقال ردیف‌ها، ابتدا یک سند ذخیره‌شده را باز کنید.', 'warning'); return; }
  if (state.dirty) {
    source = await saveVoucher({ silent: true });
    if (!source) return;
  }
  const draftSnapshot = JSON.stringify(collectDraft());
  state.busy = true;
  let next;
  try {
    next = await invoke('next_voucher_number');
  } catch (error) {
    toast(errorText(error), 'error');
    return;
  } finally {
    state.busy = false;
  }
  if (JSON.stringify(collectDraft()) !== draftSnapshot) {
    state.dirty = true;
    toast('فرم هنگام آماده‌سازی انتقال تغییر کرد؛ اطلاعات حفظ شد و عملیات باز نشد.', 'warning');
    return;
  }

  state.transfer.mode = mode; state.transfer.source = source;
  state.transfer.selectedIds = new Set();
  const target = $('#vfTransferTarget'); if (target) target.value = String(next);
  const title = $('#vfTransferTitle'), modeLabel = $('#vfTransferModeLabel');
  if (title) title.textContent = mode === 'move' ? 'انتقال ردیف‌های سند به سند دیگر' : 'کپی ردیف‌های سند به سند دیگر';
  if (modeLabel) modeLabel.textContent = mode === 'move'
    ? 'حالت عملیات: انتقال (حذف از مبدأ پس از تأیید)'
    : 'حالت عملیات: کپی (سند مبدأ حفظ می‌شود)';
  renderVoucherTransferRows();
  const modal = $('#vfTransferModal');
  transferReturnFocus = document.activeElement;
  const main = $('.voucher-window');
  transferMainWasInert = Boolean(main?.inert);
  modal.hidden = false;
  if (main) main.inert = true;
  target?.focus(); target?.select();
  void updateTransferTargetInfo();
}
function closeVoucherTransfer() {
  if (state.busy) return;
  const modal = $('#vfTransferModal'); if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window'); if (main) main.inert = transferMainWasInert;
  state.transfer.source = null; state.transfer.selectedIds = new Set();
  const returnFocus = transferReturnFocus;
  transferReturnFocus = null;
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]')) returnFocus.focus();
}
let transferTargetInfoSeq = 0;
function transferTargetNumber() {
  const text = toAsciiDigits($('#vfTransferTarget')?.value || '').trim();
  if (!/^\d+$/.test(text)) return { text, number: null };
  const number = Number(text);
  if (!Number.isSafeInteger(number) || number <= 0) return { text, number: null };
  return { text, number };
}
function setTransferTargetInfo(message, tone = '') {
  const info = $('#vfTransferTargetInfo');
  if (!info) return;
  info.textContent = message;
  if (tone) info.dataset.tone = tone;
  else delete info.dataset.tone;
}
async function updateTransferTargetInfo() {
  const { text, number } = transferTargetNumber();
  if (!text) {
    setTransferTargetInfo('');
    return;
  }
  if (number == null) {
    setTransferTargetInfo('شماره سند مقصد باید فقط شامل رقم باشد و بزرگ‌تر از صفر.', 'error');
    return;
  }
  if (!invoke) return;
  const seq = ++transferTargetInfoSeq;
  setTransferTargetInfo('در حال بررسی سند مقصد…');
  try {
    const voucher = await invoke('get_voucher', { number });
    if (seq !== transferTargetInfoSeq || !transferModalOpen()) return;
    if (voucher) {
      setTransferTargetInfo(
        `سند مقصد موجود است: شماره ${vfFa(voucher.number)} · تاریخ ${vfFa(voucher.date)} · ${voucher.type}`,
        'ok'
      );
    } else {
      setTransferTargetInfo(
        `سند شماره ${vfFa(number)} وجود ندارد؛ با اجرای عملیات، سند جدیدی با این شماره ساخته می‌شود.`,
        'info'
      );
    }
  } catch (error) {
    if (seq !== transferTargetInfoSeq) return;
    setTransferTargetInfo(`بررسی سند مقصد ناموفق بود: ${errorText(error)}`, 'error');
  }
}
async function submitVoucherTransfer() {
  if (state.busy) return;
  const source = state.transfer.source;
  const lineIds = Array.from(state.transfer.selectedIds);
  const { text: targetText, number: targetNumber } = transferTargetNumber();
  if (!source || lineIds.length < 2) { toast('حداقل دو ردیف مرتبط را انتخاب کنید.', 'warning'); return; }
  if (!targetText || targetNumber == null) {
    toast('شماره سند مقصد باید فقط شامل رقم باشد و بزرگ‌تر از صفر.', 'error'); return;
  }
  if (targetNumber === Number(source.number)) { toast('شماره سند مقصد باید با سند مبدأ متفاوت باشد.', 'error'); return; }

  state.busy = true;
  let targetVoucher;
  try {
    targetVoucher = await invoke('get_voucher', { number: targetNumber });
  } catch (error) {
    state.busy = false;
    toast(`بررسی سند مقصد ناموفق بود: ${errorText(error)}`, 'error');
    return;
  }
  state.busy = false;
  const selectionUnchanged = () => lineIds.length === state.transfer.selectedIds.size
    && lineIds.every((id) => state.transfer.selectedIds.has(id));
  if (!selectionUnchanged()) {
    toast('انتخاب ردیف‌ها حین بررسی تغییر کرد؛ عملیات متوقف شد.', 'warning');
    return;
  }

  const targetLabel = targetVoucher
    ? `سند مقصد: شماره ${vfFa(targetVoucher.number)} · تاریخ ${vfFa(targetVoucher.date)} · ${targetVoucher.type}`
    : `سند مقصد: شماره ${vfFa(targetNumber)} (سند جدید)`;
  setTransferTargetInfo(targetLabel, targetVoucher ? 'ok' : 'info');

  if (transferTargetNumber().number !== targetNumber) {
    state.busy = false;
    toast('شماره سند مقصد حین بررسی تغییر کرد؛ عملیات متوقف شد.', 'warning');
    return;
  }

  if (state.transfer.mode === 'move') {
    const ok = await confirmAction(
      `${targetLabel} · ردیف‌های انتخاب‌شده از سند مبدأ حذف و به سند مقصد افزوده می‌شوند. فقط در صورتی عملیات انجام می‌شود که هر دو سند پس از انتقال تراز بمانند. ادامه می‌دهید؟`,
      'انتقال ردیف‌ها'
    );
    if (!ok) return;
    if (transferTargetNumber().number !== targetNumber) {
      toast('شماره سند مقصد حین تأیید تغییر کرد؛ عملیات متوقف شد.', 'warning');
      return;
    }
  }
  if (!selectionUnchanged()) {
    toast('انتخاب ردیف‌ها حین تأیید تغییر کرد؛ عملیات متوقف شد.', 'warning');
    return;
  }
  const operationMode = state.transfer.mode;
  state.busy = true; setStatus('info', 'در حال کپی یا انتقال ردیف‌ها…');
  try {
    const result = await invoke('transfer_voucher_lines', { input: {
      sourceVoucherId: source.id,
      sourceVoucherNumber: source.number,
      targetVoucherNumber: targetNumber,
      lineIds, mode: operationMode,
    }});
    state.busy = false; closeVoucherTransfer(); loadVoucher(result.target);
    toast(operationMode === 'move'
      ? 'ردیف‌ها به سند شماره ' + vfFa(result.target.number) + ' منتقل شدند.'
      : 'ردیف‌ها در سند شماره ' + vfFa(result.target.number) + ' کپی شدند.', 'ok');
  } catch (error) {
    state.busy = false; setStatus('error', 'عملیات ردیف‌ها انجام نشد'); toast(errorText(error), 'error');
  }
}
function bindVoucherTransfer() {
  $('#vfTransferClose')?.addEventListener('click', closeVoucherTransfer);
  $('#vfTransferCancel')?.addEventListener('click', closeVoucherTransfer);
  $('#vfTransferSubmit')?.addEventListener('click', submitVoucherTransfer);
  $('#vfTransferTarget')?.addEventListener('change', () => void updateTransferTargetInfo());
  $('#vfTransferModal')?.addEventListener('click', (event) => {
    if (event.target.id === 'vfTransferModal') closeVoucherTransfer();
  });
  $('#vfTransferModal')?.addEventListener('keydown', (event) => {
    if (event.key !== 'Tab') return;
    const modal = $('#vfTransferModal');
    const focusable = $$('button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])', modal)
      .filter((element) => !element.hidden && element.getClientRects().length > 0);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  });
  $('#vfTransferSelectAll')?.addEventListener('click', () => {
    if (!state.transfer.source) return;
    state.transfer.selectedIds = new Set(state.transfer.source.lines.map((line) => line.id));
    renderVoucherTransferRows();
  });
  $('#vfTransferSelectNone')?.addEventListener('click', () => {
    state.transfer.selectedIds = new Set(); renderVoucherTransferRows();
  });
}
function closeExcelPreview() {
  const modal = $('#vfExcelPreviewModal'); if (!modal || modal.hidden) return;
  modal.hidden = true;
  const main = $('.voucher-window'); if (main) main.inert = excelMainWasInert;
  state.excelPreview = null;
  const returnFocus = excelReturnFocus;
  excelReturnFocus = null;
  if (returnFocus?.isConnected && !returnFocus.closest('[inert]')) returnFocus.focus();
}
function renderExcelPreview(imported) {
  const tbody = $('#vfExcelPreviewRows'), summary = $('#vfExcelPreviewSummary');
  if (!tbody || !summary) return;
  const items = imported.map((line, index) => {
    const code = digitsOnly(line.accountCode || ''), account = accountForCode(code);
    const detail1Code = digitsOnly(line.detail1Code || ''), detail = accountForCode(detail1Code);
    const debit = Number(line.debit || 0), credit = Number(line.credit || 0), errors = [];
    if (!code) errors.push('کد حساب خالی');
    else if (!account) errors.push('حساب یافت نشد');
    else if (!account.isPostable) errors.push('حساب قابل ثبت نیست');
    if (detail1Code && (!detail || Number(account?.level) !== 2 || Number(detail.level) !== 3
        || digitsOnly(detail.parentCode) !== code || String(detail.groupCode) !== String(account.groupCode)))
      errors.push('تفصیلی باید فرزند مستقیم همان حساب معین باشد');
    if ([...String(line.description || '')].length > MAX_LINE_DESCRIPTION) errors.push(`شرح بیش از ${vfFa(MAX_LINE_DESCRIPTION)} نویسه`);
    if (!Number.isSafeInteger(debit) || !Number.isSafeInteger(credit) ||
        debit < 0 || credit < 0 || debit > Number(MAX_AMOUNT) || credit > Number(MAX_AMOUNT))
      errors.push('مبلغ نامعتبر');
    else if ((debit === 0) === (credit === 0))
      errors.push('دقیقاً یکی از بدهکار/بستانکار باید مثبت باشد');
    return { index, line: { accountCode: code, detail1Code: detail1Code || null,
      description: String(line.description || ''), debit, credit }, account, detail, errors };
  });
  const validLines = items.filter((item) => !item.errors.length).map((item) => item.line);
  state.excelPreview = { lines: validLines, total: items.length, invalid: items.length - validLines.length };
  tbody.textContent = '';
  items.forEach((item) => {
    const row = document.createElement('tr'); if (item.errors.length) row.classList.add('invalid');
    const values = [vfFa(item.index + 1), vfFa(item.line.accountCode),
      item.account ? item.account.name : '—', vfFa(item.line.detail1Code || ''),
      item.detail ? item.detail.name : '—', item.line.description,
      vfFa(formatAmount(item.line.debit)), vfFa(formatAmount(item.line.credit)),
      item.errors.length ? item.errors.join('، ') : 'معتبر'];
    values.forEach((value, index) => {
      const cell = document.createElement('td'); cell.textContent = value;
      if (index === 4 || index === 5) cell.className = 'vf-money';
      if (index === 6) cell.className = item.errors.length ? 'vf-import-error' : 'vf-import-ok';
      row.append(cell);
    });
    tbody.append(row);
  });
  summary.textContent = vfFa(items.length) + ' ردیف از اکسل خوانده شد؛ ' +
    vfFa(validLines.length) + ' ردیف معتبر و ' + vfFa(state.excelPreview.invalid) +
    ' ردیف دارای خطا. فقط ردیف‌های معتبر قابل افزودن هستند.';
  $('#vfExcelAppend').disabled = !validLines.length;
  $('#vfExcelReplace').disabled = !validLines.length;
  const modal = $('#vfExcelPreviewModal');
  if (modal.hidden) {
    excelReturnFocus = document.activeElement;
    const main = $('.voucher-window');
    excelMainWasInert = Boolean(main?.inert);
    if (main) main.inert = true;
    modal.hidden = false;
    ($('#vfExcelAppend').disabled ? $('#vfExcelPreviewCancel') : $('#vfExcelAppend'))?.focus();
  }
}
async function commitExcelPreview(replaceExisting) {
  const preview = state.excelPreview; if (!preview || !preview.lines.length) return;
  const existingCount = collectDraft().lines.length;
  const resultCount = replaceExisting ? preview.lines.length : existingCount + preview.lines.length;
  if (resultCount > MAX_VOUCHER_LINES) {
    const availableCount = replaceExisting ? MAX_VOUCHER_LINES : Math.max(0, MAX_VOUCHER_LINES - existingCount);
    toast(`ظرفیت ${vfFa(availableCount)} ردیف است، اما ${vfFa(preview.lines.length)} ردیف معتبر انتخاب شده؛ هیچ تغییری اعمال نشد.`, 'warning');
    return;
  }
  if (replaceExisting && collectDraft().lines.length) {
    const ok = await confirmAction(
      'ردیف‌های فعلی سند با ردیف‌های معتبر فایل اکسل جایگزین می‌شوند. این تغییر تا زمان ذخیره سند قابل بازگشت است. ادامه می‌دهید؟',
      'جایگزینی ردیف‌ها'
    );
    if (!ok) return;
  }
  const lines = replaceExisting ? preview.lines : collectDraft().lines.concat(preview.lines);
  fillGrid(lines); state.dirty = true; closeExcelPreview(); markDirty();
  toast(vfFa(preview.lines.length) + ' ردیف معتبر از پیش‌نمایش اکسل ' +
    (replaceExisting ? 'جایگزین' : 'به سند افزوده') + ' شد.', 'ok');
}
function bindExcelPreview() {
  $('#vfExcelPreviewClose')?.addEventListener('click', closeExcelPreview);
  $('#vfExcelPreviewCancel')?.addEventListener('click', closeExcelPreview);
  $('#vfExcelAppend')?.addEventListener('click', () => commitExcelPreview(false));
  $('#vfExcelReplace')?.addEventListener('click', () => commitExcelPreview(true));
  $('#vfExcelPreviewModal')?.addEventListener('click', (event) => {
    if (event.target.id === 'vfExcelPreviewModal') closeExcelPreview();
  });
  $('#vfExcelPreviewModal')?.addEventListener('keydown', (event) => {
    if (event.key !== 'Tab') return;
    const modal = $('#vfExcelPreviewModal');
    const focusable = $$('button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])', modal)
      .filter((element) => !element.hidden && element.getClientRects().length > 0);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  });
}
function bindVoucherGridSearch() {
  $('#vfGridSearch')?.addEventListener('input', applyVoucherGridSearch);
  applyVoucherGridSearch();
}
