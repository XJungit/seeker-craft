docs: D 批次 mindcraft 对齐盘点表 — 89% 对齐，6 个缺失项

mindcraft 41 actions + 14 queries vs 当前项目 62 tools 对齐盘点：

对齐统计：
- 完全对齐: 49/55 (89%)
- 缺失: 6 个（均为低/中优先级）

缺失项：
- !newAction: 代码生成（低优先级）
- !stfu: 聊天功能（当前无聊天）
- !restart: 重启 agent（低优先级）
- !startConversation/!endConversation: bot 间对话（低优先级）
- !checkBlueprint: 整图检查（中优先级，可补）

结论：
当前项目工具数量已超过 mindcraft（62 vs 55），
核心功能 89% 对齐，缺失项不影响核心 agent 能力。
