const fs = require('fs');

// Read raw file
const profilePath = 'D:\\Craft-Agent\\profiles\\_default.json';
const raw = fs.readFileSync(profilePath, 'utf8');

// Find all unescaped quotes in the system_prompt value
const startMarker = '"system_prompt": "';
const startIdx = raw.indexOf(startMarker);
const valueStart = startIdx + startMarker.length;

// Check each character in the prompt
let issues = [];
for (let i = valueStart; i < raw.length; i++) {
  if (raw[i] === '"' && raw[i-1] !== '\\') {
    // Check if this is the end of the string
    const next = raw[i+1];
    if (next !== ',' && next !== '\n' && next !== '}' && next !== undefined) {
      issues.push({pos: i, context: raw.substring(Math.max(0,i-10), i+10)});
    }
  }
}

console.log('Found', issues.length, 'unescaped quotes');
issues.slice(0, 5).forEach(issue => {
  console.log('At', issue.pos, ':', JSON.stringify(issue.context));
});
