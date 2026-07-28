const fs = require('fs');

// Read raw file
const profilePath = 'D:\\Craft-Agent\\profiles\\_default.json';
const raw = fs.readFileSync(profilePath, 'utf8');

// Check for issues
console.log('File length:', raw.length);
console.log('Last 10 chars:', JSON.stringify(raw.substring(raw.length - 10)));

// Try to find where the JSON breaks
try {
  JSON.parse(raw);
  console.log('JSON is valid!');
} catch(e) {
  console.log('JSON error:', e.message);
  // Find the position
  const pos = e.message.match(/position (\d+)/);
  if (pos) {
    const p = parseInt(pos[1]);
    console.log('Error at position:', p);
    console.log('Context:', JSON.stringify(raw.substring(Math.max(0,p-20), p+20)));
  }
}
