const fs = require('fs');
const path = 'D:\\Craft-Agent\\config\\agent.toml';
let content = fs.readFileSync(path, 'utf8');

// Change active LLM from deepseek to agnes
content = content.replace('active = "deepseek"', 'active = "agnes"');

fs.writeFileSync(path, content);
console.log('Switched LLM backend to Agnes');
