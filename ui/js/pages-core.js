'use strict';

/* ── Page definitions ───────────────────────────────────────── */

const PAGES = {};

function pageMeta(page) {
  const titles = {
    vouchers: ['لیست اسناد حسابداری', 'عملیات'],
    journal: ['دفتر روزنامه', 'عملیات'],
    ledger: ['دفتر کل', 'گزارش‌ها'],
    trial: ['تراز آزمایشی', 'گزارش‌ها'],
    income: ['صورت سود و زیان', 'گزارش‌ها'],
    balance: ['ترازنامه', 'گزارش‌ها'],
    accounts: ['حساب‌ها', 'تعریف‌ها'],
    groups: ['گروه‌های حساب', 'تعریف‌ها'],
    settings: ['تنظیمات', 'سیستم'],
  };
  return titles[page] || ['فضای کار', ''];
}

function applyWallpaper(dataUrl) {
  const app = $('.accounting-app');
  if (!app) return;
  if (dataUrl) app.style.setProperty('--workspace-wallpaper', `url("${dataUrl}")`);
  else app.style.removeProperty('--workspace-wallpaper');
}

function chooseWallpaper(file) {
  if (!file) return;
  createImageBitmap(file).then((bitmap) => {
    const scale = Math.min(1, 2400 / bitmap.width, 1600 / bitmap.height);
    const canvas = document.createElement('canvas');
    canvas.width = Math.round(bitmap.width * scale);
    canvas.height = Math.round(bitmap.height * scale);
    canvas.getContext('2d').drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    bitmap.close();
    const dataUrl = canvas.toDataURL('image/webp', 0.82);
    const stored = storageSet('hesabdari-workspace-wallpaper', dataUrl);
    applyWallpaper(dataUrl);
    if (stored) {
      setStatus('تصویر پس‌زمینه ذخیره شد.', 'ok');
    } else {
      // The image still applies for this run; only persistence failed
      // (usually because localStorage is unavailable or full).
      toast('تصویر پس‌زمینه اعمال شد، اما ذخیره دائمی آن ممکن نشد؛ پس از بستن برنامه حفظ نمی‌شود.', 'warning');
    }
  }).catch((error) => {
    toast(`بارگذاری تصویر ناموفق بود: ${errorText(error)}`, 'error');
  });
}

const printAction = (command, argsBuilder) => ({
  label: 'چاپ گزارش',
  icon: 'i-print',
  onClick: () => printWithFeedback(command, argsBuilder()),
});

async function printWithFeedback(command, args) {
  setStatus('در حال آماده‌سازی پیش‌نمایش چاپ…', 'busy');
  try {
    await invoke(command, args);
    setStatus('پیش‌نمایش چاپ باز شد.', 'ok');
  } catch (error) {
    setStatus('آماده‌سازی گزارش ناموفق بود.', 'error');
    toast(errorText(error), 'error');
  }
}

