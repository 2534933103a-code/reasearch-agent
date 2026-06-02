# Paper Search Agent

本项目是一个智能学术论文搜索与推荐工具。

项目分为两个主要部分：

1. **Python CLI** (`arxiv_search.py`)：使用 Python 实现的快速多源论文检索工具，支持 OpenAlex, arXiv, 和 Semantic Scholar API。
2. **Paper Search 桌面端** (`paper-search/`)：一个基于 Tauri 2.x 的桌面应用（Rust 后端 + TS/HTML 前端）。它能够接收复杂的自然语言学术搜索请求，并通过 LLM 编排返回排序后、结构化的论文结果。

## 运行与编译指南

### 1. Python CLI

要求：Python 3.14，建议使用 `uv` 管理。

```bash
uv run python arxiv_search.py "<query>" [openalex|arxiv|semantic|all] [N]
# 示例: uv run python arxiv_search.py "mixture of experts inference" openalex 10
```

### 2. Paper Search (Tauri 桌面端)

进入桌面端目录：
```bash
cd paper-search
```

**开发调试**：
```bash
npm install                  # 安装前端依赖
npm run tauri dev            # 启动完整的 Tauri 应用（开发模式）
# 或者分别运行：
# npm run dev                # 启动 Vite 前端开发服务器
# cargo tauri dev            # 启动 Rust 后端
```

**编译打包**：
```bash
npm run tauri build          # 编译前端并构建 Tauri 应用程序包
# 打包 MSI (Windows):
bash scripts/build-msi.sh
```

## 架构简述

Tauri 应用的 Rust 后端采用模块化设计，工作流如下：
`Orchestrator` → `QueryDecomposer` (解析用户请求) → `SearchEngine` (多阶段检索) → `Ranker` (LLM打分过滤) → `ResultOrganizer` (生成总结与图谱数据)。

更多详细设计说明请参考 `CLAUDE.md`。