const fs = require('fs');
const c = fs.readFileSync('D:\\Craft-Agent\\profiles\\_default.json', 'utf8');
// Find system_prompt field manually
const start = c.indexOf('"system_prompt"');
if (start === -1) { console.log('NOT FOUND'); process.exit(1); }
const colon = c.indexOf(':', start);
const firstQuote = c.indexOf('"', colon + 1);
let end = firstQuote + 1;
while (end < c.length) {
  const ch = c[end];
  if (ch === '\\') { end += 2; continue; }
  if (ch === '"') { break; }
  end++;
}
const sp = c.substring(firstQuote + 1, end);
// Unescape JSON string
const unescaped = sp.replace(/\\n/g, '\n').replace(/\\"/g, '"').replace(/\\\\/g, '\\');
fs.writeFileSync('D:\\Craft-Agent\\tools\\current_prompt.txt', unescaped);
console.log('saved', unescaped.length, 'chars');
console.log('---PREVIEW---');
console.log(unescaped.substring(0, 500));
