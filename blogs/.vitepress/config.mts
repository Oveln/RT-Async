import { defineConfig } from 'vitepress'
import { withSidebar } from 'vitepress-sidebar'
import { withMermaid } from "vitepress-plugin-mermaid"

// https://vitepress.dev/reference/site-config
export default withMermaid(withSidebar(
  defineConfig({
    title: "RT-Async 博客",
    description: "基于 Rust 的 async RTOS 内核技术文档",
    themeConfig: {
      // https://vitepress.dev/reference/default-theme-config
      nav: [
        { text: '首页', link: '/' },
        { text: '文档', link: '/docs/'},
        { text: '技术报告', link: '/技术报告/' },
        { text: '周报', link: '/周报-Oveln/' },
        { text: '项目计划', link: '/项目计划/'}
      ],

      socialLinks: [
        { icon: 'github', link: 'https://github.com/Oveln/rt-async' }
      ]
    }
  }),
  {
    // 侧边栏配置
    excludeByGlobPattern: ['node_modules/**', '.vitepress/**', 'public/**'],
    sortMenusOrderByDescending: true,  // 从新到旧排序
    sortMenusByFileDatePrefix: true,   // 按文件名日期前缀排序
    sortMenusByFrontmatterDate: true,
    useTitleFromFrontmatter: true      // 从 frontmatter 获取标题
  }
))