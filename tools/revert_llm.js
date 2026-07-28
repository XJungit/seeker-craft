const fs = require('fs');
const path = 'D:\\Craft-Agent\\config\\agent.toml';
let content = fs.readFileSync(path, 'utf8');

// Revert: switch back to deepseek
content = content.replace('active = "agnes"', 'active = "deepseek"');

fs.writeFileSync(path, content);
console.log('Reverted LLM backend to deepseek');
