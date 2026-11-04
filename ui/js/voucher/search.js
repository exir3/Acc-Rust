'use strict';

/* ── Voucher search ─────────────────────────────────────────── */

let searchTimer = 0;
let searchRequest = 0;
let searchResults = [];
let searchIndex = -1;

const voucherSearchInput = () => $('#vfVoucherSearch');
const voucherSearchBox = () => $('#vfVoucherSearchResults');

const isVoucherNumberTerm = (term) => /^[0-9۰-۹٠-٩]+$/.test(toAsciiDigits(String(term)).trim());

function hideVoucherSearchResults() {
  const box = voucherSearchBox();
  const input = voucherSearchInput();
  if (box) {
    box.hidden = true;
    box.textContent = '';
  }
  if (input) input.setAttribute('aria-expanded', 'false');
  searchResults = [];
  searchIndex = -1;
}

function positionVoucherSearchResults() {
  const input = voucherSearchInput();
  const box = voucherSearchBox();
  if (!input || !box || box.hidden) return;

  const rect = input.getBoundingClientRect();
  const width = Math.max(rect.width, 360);
  let left = rect.right - width;
  if (left + width > window.innerWidth - 8) left = window.innerWidth - width - 8;
  if (left < 8) left = 8;

  box.style.width = `${width}px`;
  box.style.left = `${left}px`;
  box.style.top = `${rect.bottom + 4}px`;
}

function highlightVoucherSearchResult(index) {
  const box = voucherSearchBox();
  if (!box) return;
  const items = $$('.vf-search-result', box);
  if (!items.length) {
    searchIndex = -1;
    return;
  }
  searchIndex = (index + items.length) % items.length;
  items.forEach((item, itemIndex) => {
    item.classList.toggle('is-active', itemIndex === searchIndex);
    item.setAttribute('aria-selected', itemIndex === searchIndex ? 'true' : 'false');
  });
  items[searchIndex].scrollIntoView({ block: 'nearest' });
}

function showVoucherSearchMessage(message) {
  const box = voucherSearchBox();
  const input = voucherSearchInput();
  if (!box) return;
  box.textContent = '';
  const empty = document.createElement('div');
  empty.className = 'vf-search-result-empty';
  empty.textContent = message;
  box.append(empty);
  box.hidden = false;
  input?.setAttribute('aria-expanded', 'true');
  positionVoucherSearchResults();
}

function renderVoucherSearchResults(results) {
  const box = voucherSearchBox();
  const input = voucherSearchInput();
  if (!box) return;

  searchResults = results;
  searchIndex = results.length ? 0 : -1;
  box.textContent = '';

  if (!results.length) {
    const empty = document.createElement('div');
    empty.className = 'vf-search-result-empty';
    empty.textContent = 'سندی برای این جستجو یافت نشد.';
    box.append(empty);
  } else {
    results.forEach((voucher, index) => {
      const item = document.createElement('button');
      item.type = 'button';
      item.className = `vf-search-result${index === 0 ? ' is-active' : ''}`;
      item.dataset.index = String(index);
      item.setAttribute('role', 'option');
      item.setAttribute('aria-selected', index === 0 ? 'true' : 'false');
      item.title = `${vfFa(voucher.number)} — ${vfFa(voucher.date)} — ${voucher.type || ''}`;

      const number = document.createElement('span');
      number.className = 'vf-search-result-no';
      number.textContent = `سند ${vfFa(voucher.number)}`;

      const date = document.createElement('span');
      date.className = 'vf-search-result-date';
      date.textContent = vfFa(voucher.date || '');

      const description = document.createElement('span');
      description.className = 'vf-search-result-desc';
      description.textContent = voucher.description || voucher.type || '—';

      item.append(number, date, description);
      box.append(item);
    });
  }

  box.hidden = false;
  input?.setAttribute('aria-expanded', 'true');
  positionVoucherSearchResults();
}

async function runVoucherSearch(term) {
  if (!invoke) return;
  const wanted = term.trim();
  if (!wanted) {
    hideVoucherSearchResults();
    return;
  }

  const request = ++searchRequest;
  try {
    const payload = await invoke('list_vouchers', { from: '', to: '', search: wanted });
    if (request !== searchRequest) return;
    if (normalizeSearch(voucherSearchInput()?.value || '') !== normalizeSearch(wanted)) return;

    const allResults = Array.isArray(payload?.vouchers) ? payload.vouchers : [];
    let results;
    if (isVoucherNumberTerm(wanted)) {
      const exactNumber = digitsOnly(wanted);
      const exact = allResults.find((voucher) => String(voucher.number) === exactNumber);
      results = exact
        ? [exact, ...allResults.filter((voucher) => voucher !== exact).slice(0, 7)]
        : allResults.slice(0, 8);
    } else {
      results = allResults.slice(0, 8);
    }
    renderVoucherSearchResults(results);
  } catch (error) {
    if (request !== searchRequest) return;
    showVoucherSearchMessage(errorText(error));
  }
}

function chooseVoucherSearchResult(index = searchIndex) {
  const voucher = searchResults[index];
  if (!voucher) return;
  hideVoucherSearchResults();
  const input = voucherSearchInput();
  if (input) input.value = '';
  openNumber(voucher.number);
}

function bindVoucherSearch() {
  const input = voucherSearchInput();
  const box = voucherSearchBox();
  if (!input || !box) return;

  const search = () => {
    const term = input.value.trim();
    window.clearTimeout(searchTimer);
    searchRequest += 1;
    if (!term) {
      hideVoucherSearchResults();
      return;
    }
    searchTimer = window.setTimeout(() => runVoucherSearch(term), 180);
  };

  input.addEventListener('input', search);
  input.addEventListener('focus', () => {
    if (input.value.trim() && searchResults.length) {
      box.hidden = false;
      input.setAttribute('aria-expanded', 'true');
      positionVoucherSearchResults();
    }
  });

  input.addEventListener('keydown', (ev) => {
    if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
      ev.preventDefault();
      if (box.hidden) {
        if (input.value.trim()) runVoucherSearch(input.value.trim());
        return;
      }
      highlightVoucherSearchResult(searchIndex + (ev.key === 'ArrowDown' ? 1 : -1));
      return;
    }

    if (ev.key === 'Enter') {
      ev.preventDefault();
      const term = input.value.trim();
      if (!term) return;
      if (isVoucherNumberTerm(term)) {
        hideVoucherSearchResults();
        input.value = '';
        openNumber(digitsOnly(term));
        return;
      }
      if (searchResults.length) {
        chooseVoucherSearchResult();
        return;
      }
      runVoucherSearch(term);
      return;
    }

    if (ev.key === 'Escape' && !box.hidden) {
      ev.preventDefault();
      hideVoucherSearchResults();
    }
  });

  box.addEventListener('mousedown', (ev) => ev.preventDefault());
  box.addEventListener('click', (ev) => {
    const item = ev.target.closest('.vf-search-result');
    if (!item) return;
    chooseVoucherSearchResult(Number(item.dataset.index));
  });

  document.addEventListener('pointerdown', (ev) => {
    if (box.hidden) return;
    if (box.contains(ev.target) || input === ev.target) return;
    hideVoucherSearchResults();
  });

  window.addEventListener('resize', positionVoucherSearchResults);
  $('.vf-meta')?.addEventListener('scroll', positionVoucherSearchResults, { passive: true });
}
