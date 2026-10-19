import { expect, it } from 'vitest'

import i18n, { getSystemResolvedLocale, initializeLocale, setAppLocale } from './i18n.config'

it('starts with no eager language dictionaries and loads the system language with English fallback', async () => {
    expect(i18n.global.availableLocales).toEqual([])
    const systemLocale = getSystemResolvedLocale()
    await initializeLocale()
    expect(i18n.global.availableLocales.sort()).toEqual(
        [...new Set(['en-US', systemLocale])].sort(),
    )
    expect(i18n.global.locale.value).toBe(systemLocale)
    expect(Object.keys(i18n.global.getLocaleMessage('en-US')).length).toBeGreaterThan(0)
    await setAppLocale('zh-CN')
    expect(i18n.global.locale.value).toBe('zh-CN')
    expect(Object.keys(i18n.global.getLocaleMessage('zh-CN')).length).toBeGreaterThan(0)
    await setAppLocale('not-a-locale')
    expect(i18n.global.locale.value).toBe('en-US')
})
