const fs = require('fs');
const path = require('path');
// 从脚本位置推导仓库根：scripts/llm/switch_llm.js -> <repo>/data/config/agent.toml
const repoRoot = path.resolve(__dirname, '..', '..');
const file = path.join(repoRoot, 'data', 'config', 'agent.toml');
let content = fs.readFileSync(file, 'utf8');

// Change active LLM from deepseek to agnes
content = content.replace('active = "deepseek"', 'active = "agnes"');

fs.writeFileSync(file, content);
console.log('Switched LLM backend to Agnes');
