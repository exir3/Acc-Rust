import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import vm from 'node:vm';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const corePath = path.join(projectRoot, 'ui', 'js', 'voucher', 'account-picker-core.js');
const sandbox = {};
vm.runInNewContext(readFileSync(corePath, 'utf8'), sandbox, { filename: corePath });
const core = sandbox.AccountPickerCore;

const accounts = [
  { code: '1000', name: 'کل دارای معین', level: 1, parentCode: '' },
  { code: '2000', name: 'کل بدون معین', level: 1, parentCode: '' },
  { code: '100001', name: 'معین دارای تفصیلی', level: 2, parentCode: '1000' },
  { code: '100002', name: 'معین بدون تفصیلی', level: 2, parentCode: '1000' },
  { code: '200001', name: 'معین دوم', level: 2, parentCode: '2000' },
  { code: '100001001', name: 'تفصیلی یک', level: 3, parentCode: '100001' },
  { code: '100001002', name: 'تفصیلی دو', level: 3, parentCode: '100001' },
];

const codes = (items) => Array.from(items, (account) => account.code);

test('root of the base picker lists only کل accounts', () => {
  assert.deepEqual(
    codes(core.visibleAccounts(accounts, { mode: 'base', path: [], showAll: false })),
    ['1000', '2000'],
  );
});

test('drilling one level lists only direct children of the selected کل', () => {
  assert.deepEqual(
    codes(core.visibleAccounts(accounts, {
      mode: 'base',
      path: [accounts[0]],
      showAll: false,
    })),
    ['100001', '100002'],
  );
});

test('drilling a معین lists only its direct تفصیلی ۱ children', () => {
  assert.deepEqual(
    codes(core.visibleAccounts(accounts, {
      mode: 'base',
      path: [accounts[0], accounts[2]],
      showAll: false,
    })),
    ['100001001', '100001002'],
  );
});

test('show all returns a flat کل/معین list and ignores the drill path', () => {
  assert.deepEqual(
    codes(core.visibleAccounts(accounts, {
      mode: 'base',
      path: [accounts[0]],
      showAll: true,
    })),
    ['1000', '2000', '100001', '100002', '200001'],
  );
});

test('detail mode lists only details of the selected معین', () => {
  assert.deepEqual(
    codes(core.visibleAccounts(accounts, { mode: 'detail', baseCode: '100001' })),
    ['100001001', '100001002'],
  );
  assert.deepEqual(
    codes(core.visibleAccounts(accounts, { mode: 'detail', baseCode: '' })),
    [],
  );
});

test('search pool stays independent of the drill level', () => {
  assert.deepEqual(
    codes(core.searchPool(accounts, { mode: 'base' })),
    ['1000', '2000', '100001', '100002', '200001'],
  );
  assert.deepEqual(
    codes(core.searchPool(accounts, { mode: 'detail', baseCode: '100001' })),
    ['100001001', '100001002'],
  );
});

test('only accounts with children can be drilled into', () => {
  assert.equal(core.canDrill(accounts, accounts[0], 'base'), true);
  assert.equal(core.canDrill(accounts, accounts[2], 'base'), true);
  assert.equal(core.canDrill(accounts, accounts[3], 'base'), false);
  assert.equal(core.canDrill(accounts, accounts[5], 'base'), false);
  assert.equal(core.canDrill(accounts, accounts[0], 'detail'), false);
  assert.equal(core.hasChildren(accounts, accounts[5]), false);
});

test('ancestors resolve the full path from root to the account parent', () => {
  assert.deepEqual(codes(core.ancestors(accounts[5], accounts)), ['1000', '100001']);
  assert.deepEqual(codes(core.ancestors(accounts[2], accounts)), ['1000']);
  assert.deepEqual(codes(core.ancestors(accounts[0], accounts)), []);
});

test('Persian digits and malformed parent codes are normalised safely', () => {
  assert.deepEqual(
    codes(core.directChildren(accounts, '۱۰۰۰', 1)),
    ['100001', '100002'],
  );
  assert.deepEqual(codes(core.directChildren(accounts, '', 1)), []);
  assert.deepEqual(
    codes(core.visibleAccounts(accounts, {
      mode: 'base',
      path: [{ code: '100001', level: 3, parentCode: '1000' }],
      showAll: false,
    })),
    [],
  );
});
