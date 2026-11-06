'use strict';

/* ── Reusable row/document descriptions ─────────────────────── */

const SAVED_DESCRIPTIONS_KEY = 'saved_descriptions:document-form';
let savedDescriptionTarget = null;
let savedDescriptionEditIndex = -1;

function readSavedDescriptions() {
  try {
    const value = JSON.parse(storageGet(SAVED_DESCRIPTIONS_KEY) || '[]');
    return Array.isArray(value)
      ? value.filter((item) => typeof item === 'string' && item.trim()).slice(0, 500)
      : [];
  } catch {
    return [];
  }
}

function writeSavedDescriptions(items) {
  try {
    if (!storageSet(SAVED_DESCRIPTIONS_KEY, JSON.stringify(items.slice(0, 500)))) {
      throw new Error('storage unavailable');
    }
    return true;
  } catch {
    toast('ذخیره شرح‌ها در حافظه محلی مرورگر ممکن نشد.', 'error');
    return false;
  }
}

function closeSavedDescriptions() {
  const modal = $('#vfSavedDescriptionsModal');
  if (!modal) return;
  modal.hidden = true;
  const target = savedDescriptionTarget;
  savedDescriptionTarget = null;
  savedDescriptionEditIndex = -1;
  if (target?.isConnected) target.focus();
}

function applySavedDescription(value) {
  const target = savedDescriptionTarget;
  if (!target) return;
  const limit = target.classList.contains('vf-desc-input') ? MAX_LINE_DESCRIPTION : MAX_DESCRIPTION;
  const characters = Array.from(value);
  target.value = characters.slice(0, limit).join('');
  if (characters.length > limit) {
    toast(`شرح انتخاب‌شده به سقف ${vfFa(limit)} نویسه محدود شد.`, 'info');
  }
  target.dispatchEvent(new Event('input', { bubbles: true }));
  target.dispatchEvent(new Event('change', { bubbles: true }));
  markDirty();
  closeSavedDescriptions();
}

function renderSavedDescriptionRows() {
  const box = $('#vfSavedDescriptionRows');
  if (!box) return;
  const items = readSavedDescriptions();
  box.textContent = '';
  if (!items.length) {
    const empty = document.createElement('div');
    empty.className = 'vf-saved-description-empty';
    empty.textContent = 'هنوز شرح ذخیره‌شده‌ای تعریف نشده است.';
    box.append(empty);
    return;
  }
  items.forEach((value, index) => {
    const item = document.createElement('div');
    item.className = 'vf-saved-description-row';
    item.setAttribute('role', 'option');
    item.setAttribute('aria-selected', 'false');
    const text = document.createElement('span');
    text.className = 'vf-saved-description-text';
    text.textContent = value;
    const choose = document.createElement('button');
    choose.type = 'button';
    choose.className = 'ap-tool ap-tool-select';
    choose.textContent = 'انتخاب';
    choose.addEventListener('click', () => applySavedDescription(value));
    const edit = document.createElement('button');
    edit.type = 'button';
    edit.className = 'ap-tool';
    edit.textContent = 'ویرایش';
    edit.addEventListener('click', () => {
      savedDescriptionEditIndex = index;
      $('#vfSavedDescriptionInput').value = value;
      $('#vfSavedDescriptionUpdate').disabled = false;
      $('#vfSavedDescriptionInput').focus();
      $('#vfSavedDescriptionInput').select();
    });
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'ap-tool';
    remove.textContent = 'حذف';
    remove.addEventListener('click', () => {
      const next = readSavedDescriptions();
      next.splice(index, 1);
      if (writeSavedDescriptions(next)) {
        savedDescriptionEditIndex = -1;
        $('#vfSavedDescriptionUpdate').disabled = true;
        renderSavedDescriptionRows();
      }
    });
    item.append(text, choose, edit, remove);
    box.append(item);
  });
}

function openSavedDescriptions(row = null) {
  if (isLockedVoucher()) {
    toast('سند دائمی است و امکان تغییر شرح آن وجود ندارد.', 'info');
    return;
  }
  const modal = $('#vfSavedDescriptionsModal');
  const input = $('#vfSavedDescriptionInput');
  if (!modal || !input) return;
  savedDescriptionTarget = row?.querySelector('.vf-desc-input') || $('#vfDocDesc');
  savedDescriptionEditIndex = -1;
  $('#vfSavedDescriptionUpdate').disabled = true;
  input.value = savedDescriptionTarget?.value || '';
  renderSavedDescriptionRows();
  modal.hidden = false;
  input.focus();
  input.select();
}

function bindSavedDescriptions() {
  const modal = $('#vfSavedDescriptionsModal');
  if (!modal) return;
  $('#vfSavedDescriptionsClose')?.addEventListener('click', closeSavedDescriptions);
  $('#vfSavedDescriptionsCancel')?.addEventListener('click', closeSavedDescriptions);
  modal.addEventListener('click', (event) => {
    if (event.target === modal) closeSavedDescriptions();
  });
  $('#vfSavedDescriptionAdd')?.addEventListener('click', () => {
    const input = $('#vfSavedDescriptionInput');
    const value = input?.value.trim() || '';
    if (!value) {
      toast('شرح را وارد کنید.', 'info');
      input?.focus();
      return;
    }
    const items = readSavedDescriptions();
    if (items.some((item) => item === value)) {
      toast('این شرح قبلاً ذخیره شده است.', 'info');
      return;
    }
    items.unshift(value);
    if (writeSavedDescriptions(items)) {
      input.value = '';
      savedDescriptionEditIndex = -1;
      $('#vfSavedDescriptionUpdate').disabled = true;
      renderSavedDescriptionRows();
      input.focus();
    }
  });
  $('#vfSavedDescriptionUpdate')?.addEventListener('click', () => {
    const input = $('#vfSavedDescriptionInput');
    const value = input?.value.trim() || '';
    if (savedDescriptionEditIndex < 0 || !value) return;
    const items = readSavedDescriptions();
    if (items.some((item, index) => index !== savedDescriptionEditIndex && item === value)) {
      toast('شرح مشابهی در فهرست وجود دارد.', 'info');
      return;
    }
    items[savedDescriptionEditIndex] = value;
    if (writeSavedDescriptions(items)) {
      savedDescriptionEditIndex = -1;
      $('#vfSavedDescriptionUpdate').disabled = true;
      renderSavedDescriptionRows();
    }
  });
  modal.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeSavedDescriptions();
    } else if (event.key === 'Enter' && event.target === $('#vfSavedDescriptionInput')) {
      event.preventDefault();
      $('#vfSavedDescriptionAdd')?.click();
    }
  });
}
