const fs = require('fs');

// Read raw file
const profilePath = 'D:\\Craft-Agent\\profiles\\_default.json';
let raw = fs.readFileSync(profilePath, 'utf8');

// Fix: escape inner double quotes in the chat command
// The issue is: chat("/tp @s ~ 70 ~") - the inner quotes need to be escaped
raw = raw.replace('chat("/tp @s ~ 70 ~")', 'chat(\\"/tp @s ~ 70 ~\")');

// Also fix any other unescaped quotes in the prompt
// Find all instances of ("/ that are not already escaped
raw = raw.replace(/(?<!\\)("\/)/g, '\\"$1');

fs.writeFileSync(profilePath, raw);

// Verify
try {
  const j = JSON.parse(raw);
  console.log('JSON is now valid!');
  console.log('Prompt length:', j.system_prompt.length);
  console.log('First 100 chars:', j.system_prompt.substring(0, 100));
} catch(e) {
  console.log('Still invalid:', e.message);
}
