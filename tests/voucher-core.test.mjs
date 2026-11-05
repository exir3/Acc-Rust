import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import vm from 'node:vm';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sandbox = { window: {} };
vm.createContext(sandbox);

for (const relative of ['ui/js/voucher/core.js', 'ui/js/voucher/amounts.js']) {
  const file = path.join(projectRoot, relative);
  vm.runInContext(readFileSync(file, 'utf8'), sandbox, { filename: file });
}

const call = (expression) => vm.runInContext(expression, sandbox);

test('Persian and Arabic-Indic digits convert to ASCII', () => {
  assert.equal(call("toAsciiDigits('۱۲۳٤٥٦')"), '123456');
  assert.equal(call("digitsOnly('۱۲,۳۴۵ ریال')"), '12345');
  assert.equal(call("digitsOnly('abc')"), '');
});

test('search normalisation ignores digit script, Arabic yeh/kaf and ZWNJ', () => {
  assert.equal(call("normalizeSearch('حساب كلي')"), 'حساب کلی');
  assert.equal(call("normalizeSearch('۱۲۳')"), '123');
  assert.equal(call("normalizeSearch('  كل   معين ')"), 'کل معین');
  assert.equal(call("normalizeSearch('')"), '');
});

test('amount formatting groups thousands and keeps negatives', () => {
  assert.equal(call('formatAmount(1234567n)'), '1,234,567');
  assert.equal(call("formatAmount('۱۲۳۴')"), '1,234');
  assert.equal(call('formatAmount(0)'), '0');
  assert.equal(call('formatAmount(-2500n)'), '-2,500');
});

test('amount parsing accepts grouped digits and rejects malformed input', () => {
  assert.equal(call("String(parseAmountInput('1,234,567'))"), '1234567');
  assert.equal(call("String(parseAmountInput('۱۲۳۴'))"), '1234');
  assert.equal(call("String(parseAmountInput(''))"), '0');
  assert.equal(call("String(parseAmountInput('1,23'))"), 'null');
  assert.equal(call("String(parseAmountInput('12.5'))"), 'null');
  assert.equal(call("String(parseAmountInput('1,234.5'))"), 'null');
});

test('caret position stays after the typed digits while grouping', () => {
  assert.equal(call("amountCaretPosition('1,234', 4)"), 5);
  assert.equal(call("amountCaretPosition('1,234', 0)"), 0);
  assert.equal(call("amountCaretPosition('1,234', 9)"), 5);
});

test('Jalali month lengths follow the 33-year leap rule', () => {
  assert.equal(call('isJalaliLeapYear(1403)'), true);
  assert.equal(call('isJalaliLeapYear(1402)'), false);
  assert.equal(call('jalaliMonthLength(1403, 1)'), 31);
  assert.equal(call('jalaliMonthLength(1403, 7)'), 30);
  assert.equal(call('jalaliMonthLength(1403, 12)'), 30);
  assert.equal(call('jalaliMonthLength(1402, 12)'), 29);
  assert.equal(call('jalaliMonthLength(1403, 13)'), 0);
});
