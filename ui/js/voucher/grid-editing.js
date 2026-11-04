'use strict';

/* ── Grid editing ───────────────────────────────────────────── */

function bindVoucherGrid() {
  const tbody = $('#voucherRows');
  if (!tbody) return;

  tbody.addEventListener('focusin', (ev) => {
    const row = ev.target.closest('tr');
    if (!row) return;
    state.activeRow = row;
    const input = ev.target;
    if (input.classList.contains('vf-amount-input')) {
      // Drop the thousands separators while editing.
      input.value = digitsOnly(input.value);
    }
    updateRowAccount(row, input.classList.contains('vf-code-input') || input.classList.contains('vf-detail1-input'));
    updateLedger(row);
    updateTotals();
  });

  tbody.addEventListener('focusout', (ev) => {
    const input = ev.target;
    if (input.classList && input.classList.contains('vf-amount-input') && input.value.trim() !== '') {
      input.value = formatAmount(parseAmount(input.value));
    }
    if (input.classList && (input.classList.contains('vf-code-input') || input.classList.contains('vf-detail1-input'))) {
      updateRowAccount(input.closest('tr'), true);
    }
  });

  tbody.addEventListener('input', (ev) => {
    const input = ev.target;
    if (!input.matches('input')) return;

    if (input.classList.contains('vf-code-input') || input.classList.contains('vf-amount-input')) {
      const rawValue = toAsciiDigits(input.value);
      if (input.classList.contains('vf-amount-input') && /[-−]/.test(rawValue)) {
        input.value = '';
        toast('مبلغ منفی مجاز نیست؛ از ستون مقابل استفاده کنید.', 'error');
      } else if (input.classList.contains('vf-amount-input') && /[./٫]/.test(rawValue)) {
        input.value = '';
        toast('مبلغ باید عدد صحیح ریالی باشد؛ اعشار مجاز نیست.', 'error');
      }
      let cleaned = digitsOnly(input.value);
      if (input.classList.contains('vf-amount-input') && parseAmount(cleaned) > MAX_AMOUNT) {
        input.value = cleaned;
        input.classList.add('is-invalid');
        input.setAttribute('aria-invalid', 'true');
        toast(`مبلغ هر ردیف حداکثر ${vfFa(formatAmount(MAX_AMOUNT))} ریال می‌تواند باشد.`, 'error');
      } else {
        if (input.classList.contains('vf-amount-input')) {
          input.classList.remove('is-invalid');
          input.removeAttribute('aria-invalid');
        }
        if (input.value !== cleaned) input.value = cleaned;
      }
    }

    const row = input.closest('tr');
    if (!row) return;

    if (input.classList.contains('vf-code-input')) {
      updateRowAccount(row);
      if (state.activeRow === row) updateLedger(row);
    }

    // Debit and credit are mutually exclusive on one line.
    if (input.classList.contains('vf-amount-input') && input.value !== '') {
      const other = input.classList.contains('vf-debit-input')
        ? row.querySelector('.vf-credit-input')
        : row.querySelector('.vf-debit-input');
      if (other && other.value !== '') other.value = '';
    }

    ensureTrailingBlank();
    refreshFormState();
  });

  tbody.addEventListener('dblclick', (ev) => {
    const input = ev.target.closest('.vf-code-input, .vf-detail1-input');
    if (input) {
      ev.preventDefault();
      openAccountPicker(input);
    }
  });

  tbody.addEventListener('keydown', (ev) => {
    const input = ev.target;
    if (!input.matches || !input.matches('input')) return;

    const isSpace = ev.code === 'Space' || ev.key === ' ' || ev.key === 'Spacebar';
    if ((isSpace || ev.key === 'F4') && (input.classList.contains('vf-code-input') || input.classList.contains('vf-detail1-input'))) {
      ev.preventDefault();
      openAccountPicker(input);
      return;
    }

    if (ev.key === 'Enter' && !ev.ctrlKey && !ev.altKey && !ev.metaKey) {
      ev.preventDefault();
      moveGridFocus(input, input.closest('tr'), ev.shiftKey ? -1 : 1);
    }
  });

  window.addEventListener('resize', () => {
    syncVoucherRowCount();
    refreshFormState();
  });

  const form = $('.voucher-form');
  if (form) {
    const markFormDirty = (ev) => {
      // Typing in the voucher search box does not change the voucher.
      if (ev.target && ev.target.id === 'vfVoucherSearch') return;
      markDirty();
    };
    form.addEventListener('input', markFormDirty);
    form.addEventListener('change', markFormDirty);
  }
}

// Enter walks the row: base account → detail-1 → description → debit → credit.
function moveGridFocus(input, row, step) {
  if (!row) return;
  const cells = [
    row.querySelector('.vf-code-input'),
    row.querySelector('.vf-detail1-input'),
    row.querySelector('.vf-desc-input'),
    row.querySelector('.vf-debit-input'),
    row.querySelector('.vf-credit-input'),
  ].filter(Boolean);
  const index = cells.indexOf(input);
  if (index === -1) return;

  const nextIndex = index + step;
  if (nextIndex >= 0 && nextIndex < cells.length) {
    cells[nextIndex].focus();
    return;
  }

  if (step > 0) {
    ensureTrailingBlank();
    const nextRow = row.nextElementSibling;
    if (nextRow) nextRow.querySelector('.vf-code-input')?.focus();
    else if (gridRows().filter(rowHasValue).length >= MAX_VOUCHER_LINES) {
      toast(`حداکثر ${vfFa(MAX_VOUCHER_LINES)} ردیف برای هر سند مجاز است.`, 'warning');
    }
  } else {
    const prevRow = row.previousElementSibling;
    if (prevRow) prevRow.querySelector('.vf-credit-input')?.focus();
  }
}

