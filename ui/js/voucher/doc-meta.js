'use strict';

/* ── Document meta and description ──────────────────────────── */

function updateDescCounter() {
  const area = $('#vfDocDesc');
  const output = $('#vfDocDescCount');
  if (!area || !output) return;
  let chars = Array.from(area.value);
  if (chars.length > MAX_DESCRIPTION) {
    area.value = chars.slice(0, MAX_DESCRIPTION).join('');
    chars = chars.slice(0, MAX_DESCRIPTION);
  }
  output.textContent = `${vfFa(chars.length)}/${vfFa(MAX_DESCRIPTION)}`;
}

function bindDocMeta() {
  const docNo = $('#vfDocNo');
  const date = $('#vfDocDate');
  const desc = $('#vfDocDesc');

  if (docNo) {
    docNo.addEventListener('input', () => {
      const cleaned = digitsOnly(docNo.value);
      if (docNo.value !== cleaned) docNo.value = cleaned;
    });
    docNo.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') {
        ev.preventDefault();
        openNumber(digitsOnly(docNo.value));
      }
    });
  }

  if (date) {
    let validationSequence = 0;
    date.addEventListener('input', () => {
      validationSequence += 1;
      date.value = toAsciiDigits(date.value).replace(/-/g, '/').replace(/[^\d/]/g, '').slice(0, 10);
      date.classList.remove('is-invalid');
      date.removeAttribute('aria-invalid');
      date.title = '';
    });
    date.addEventListener('blur', async () => {
      const value = toAsciiDigits(date.value).replace(/-/g, '/').replace(/[^\d/]/g, '');
      if (!value || !invoke) return;
      const request = ++validationSequence;
      try {
        const normalizedDate = await invoke('validate_voucher_date', { date: value });
        if (request !== validationSequence || date.value !== value) return;
        if (typeof normalizedDate === 'string') date.value = normalizedDate;
        date.classList.remove('is-invalid');
        date.removeAttribute('aria-invalid');
        date.title = '';
      } catch (error) {
        if (request !== validationSequence || date.value !== value) return;
        date.classList.add('is-invalid');
        date.setAttribute('aria-invalid', 'true');
        date.title = errorText(error);
        toast(errorText(error), 'error');
      }
    });
  }

  if (desc) desc.addEventListener('input', updateDescCounter);

  const permanentToggle = $('#vfPermanentToggle');
  if (permanentToggle) {
    permanentToggle.addEventListener('change', async () => {
      if (isLockedVoucher()) {
        permanentToggle.checked = true;
        return;
      }
      if (permanentToggle.checked) {
        const confirmed = await confirmAction(
          'پس از ذخیره، این سند دائمی و فقط‌خواندنی می‌شود. ویرایش، حذف یا بازگرداندن آن به وضعیت موقت ممکن نخواهد بود. ادامه می‌دهید؟',
          'تأیید دائمی‌سازی'
        );
        if (!confirmed) {
          permanentToggle.checked = false;
          state.makePermanent = false;
          refreshFormState();
          return;
        }
        state.makePermanent = true;
      } else {
        state.makePermanent = false;
      }
      markDirty();
      refreshFormState();
    });
  }

  $('#vfPrevBtn')?.addEventListener('click', navigatePrev);
  $('#vfNextBtn')?.addEventListener('click', navigateNext);
}

