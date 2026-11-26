import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitepress'
import { rewriteLinks } from './links.ts'
import { buildNavigation } from './navigation.ts'

const repository = 'https://github.com/Arkazzae/Stellarr'
const docs = fileURLToPath(new URL('..', import.meta.url))
const logos = fileURLToPath(new URL('../../website/src/images/logo', import.meta.url))
const fonts = fileURLToPath(new URL('../../web/public/fonts', import.meta.url))

// Embed the same logo files that ship with the project website.
function drawing(name: string): string {
  const svg = readFileSync(`${logos}/${name}.svg`, 'utf8')
  return `data:image/svg+xml,${encodeURIComponent(svg)}`
}

const { nav, sidebar } = buildNavigation(docs)

export default defineConfig({
  title: 'Stellarr',
  description: 'Documentation of Stellarr, a self-hosted text and voice chat for a small group of friends.',
  lang: 'en',
  base: '/Stellar/',
  cleanUrls: true,
  lastUpdated: true,
  // The documentation map is the home page; the file stays README.md for GitHub.
  rewrites: { 'README.md': 'index.md' },
  head: [['link', { rel: 'icon', type: 'image/svg+xml', href: drawing('tile') }]],
  markdown: {
    config(md) {
      rewriteLinks(repository)(md)
      const defaultFence = md.renderer.rules.fence!
      md.renderer.rules.fence = (tokens, idx, options, env, self) => {
        const token = tokens[idx]
        if (token.info.trim() === 'mermaid') {
          return `<MermaidDiagram code="${encodeURIComponent(token.content)}" />`
        }
        return defaultFence(tokens, idx, options, env, self)
      }
    },
  },
  transformHead({ assets }) {
    const outfit = assets.find((file) => /outfit-100-900-latin\.[\w-]+\.woff2$/.test(file))
    if (!outfit) return []
    return [['link', { rel: 'preload', href: outfit, as: 'font', type: 'font/woff2', crossorigin: '' }]]
  },
  vite: {
    // The style sheet loads the fonts from the web client, outside the docs/ root.
    server: { fs: { allow: [docs, fonts] } },
  },
  themeConfig: {
    // small.svg (the mark for 16 to 47 px) has a light bubble for dark
    // backgrounds. The light scheme takes the one-colour mark, which an image
    // draws in black.
    logo: { light: drawing('mono'), dark: drawing('small') },
    nav,
    sidebar,
    outline: { level: [2, 3] },
    socialLinks: [{ icon: 'github', link: repository, ariaLabel: 'Stellarr on GitHub' }],
    editLink: {
      pattern: `${repository}/edit/main/docs/:path`,
      text: 'Edit this page on GitHub',
    },
    search: { provider: 'local' },
  },
})
