const fs = require('fs');
const path = require('path');
// 从脚本位置推导仓库根：scripts/llm/revert_llm.js -> <repo>/data/config/agent.toml
const repoRoot = path.resolve(__dirname, '..', '..');
const file = path.join(repoRoot, 'data', 'config', 'agent.toml');
let content = fs.readFileSync(file, 'utf8');

// Revert: switch back to deepseek
content = content.replace('active = "agnes"', 'active = "deepseek"');

fs.writeFileSync(file, content);
console.log('Reverted LLM backend to deepseek');
