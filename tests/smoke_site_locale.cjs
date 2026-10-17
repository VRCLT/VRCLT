// Entry routing must respect browser preference order and explicit choices.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../site/locale.js'), 'utf8');

function visit({languages = ['en-US'], saved, root = true, blocked = false, version = null,
                href = 'https://vrclt.github.io/'} = {}) {
  const redirects = [], writes = [], events = {}, cleanups = [];
  const url = new URL(href);
  vm.runInNewContext(source, {
    URL,
    navigator: {languages, language: languages[0]},
    document: {
      documentElement: {hasAttribute: name => root && name === 'data-auto-language',
        getAttribute: name => name === 'data-site-version' ? version : null},
      addEventListener: (name, handler) => { events[name] = handler; },
    },
    location: {href, search: url.search, hash: url.hash, replace: next => redirects.push(next)},
    history: {state:null, replaceState: (state, title, next) => cleanups.push(next)},
    localStorage: {
      getItem: () => { if (blocked) throw Error('disabled'); return saved; },
      setItem: (key, value) => { if (blocked) throw Error('disabled'); writes.push({key, value}); },
    },
  });
  return {redirects, writes, cleanups, click(language) {
    events.click({target: {closest: () => language ? {dataset: {language}} : null}});
  }};
}

const cases = [
  [['ko-KR', 'en-US'], 'ko'], [['en-GB', 'ko'], 'en'], [['ja-JP'], 'ja'],
  [['zh-CN'], 'zh'], [['zh-SG'], 'zh'], [['zh'], 'zh'],
  [['zh-TW'], 'zh-hant'], [['zh-HK'], 'zh-hant'], [['zh-MO'], 'zh-hant'],
  [['zh-Hant'], 'zh-hant'], [['zh-Hant-CN'], 'zh-hant'], [['zh-Hans-TW'], 'zh'],
  [['ZH_hk'], 'zh-hant'], [['de-DE', 'ja-JP'], 'ja'], [['fr-FR'], 'en'], [[], 'en'],
];
for (const [languages, expected] of cases) {
  const result = visit({languages});
  assert.deepEqual(result.redirects, [`https://vrclt.github.io/${expected}/`]);
  assert.deepEqual(result.writes, [], 'Automatic detection must not overwrite a manual preference.');
}
assert.deepEqual(visit({languages:['ko'], saved:'en'}).redirects, ['https://vrclt.github.io/en/']);
assert.deepEqual(visit({languages:['en'], saved:'zh-hant'}).redirects, ['https://vrclt.github.io/zh-hant/']);
assert.deepEqual(visit({languages:['ko'], saved:'invalid'}).redirects, ['https://vrclt.github.io/ko/']);
assert.deepEqual(visit({languages:['ko'], blocked:true}).redirects, ['https://vrclt.github.io/ko/']);
assert.deepEqual(visit({languages:['ko'], href:'https://vrclt.github.io/index.html?ref=readme#start'}).redirects,
  ['https://vrclt.github.io/ko/?ref=readme#start']);

for (const language of ['en', 'ko', 'ja', 'zh', 'zh-hant']) {
  const direct = visit({root:false, href:`https://vrclt.github.io/${language}/`, saved:'ko'});
  assert.deepEqual(direct.redirects, [], 'Explicit URLs must not redirect or loop.');
  direct.click(language);
  assert.deepEqual(direct.writes, [{key:'vrclt.site.language', value:language}]);
}
const noChoice = visit({root:false});
noChoice.click(null);
noChoice.click('invalid');
assert.deepEqual(noChoice.writes, []);
assert.doesNotThrow(() => visit({root:false, blocked:true}).click('ja'));
assert.deepEqual(visit({languages:['ko'], version:'abc123', href:'https://vrclt.github.io/?ref=readme#start'}).redirects,
  ['https://vrclt.github.io/ko/?ref=readme&v=abc123#start']);
assert.deepEqual(visit({root:false, version:'abc123', href:'https://vrclt.github.io/ko/?v=abc123&ref=readme#start'}).cleanups,
  ['https://vrclt.github.io/ko/?ref=readme#start']);
console.log('Locale routing: 16 language cases, preference precedence, direct links, storage denial, and manual selection passed.');
