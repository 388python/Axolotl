import {
    buildLocaleMessages,
    createMessageCompiler,
    type CrowdinMessages,
    uiLocaleModules,
} from '@modrinth/ui'
import { createI18n } from 'vue-i18n'

import { createLocaleLoader } from '@/helpers/locale-loader'

const localeModules = import.meta.glob<{ default: CrowdinMessages }>('./locales/*/index.json')
const availableLocales = Object.keys(localeModules).map((path) => path.split('/').at(-2)!)

const i18n = createI18n({
    legacy: false,
    locale: 'en-US',
    fallbackLocale: 'en-US',
    messageCompiler: createMessageCompiler(),
    missingWarn: false,
    fallbackWarn: false,
    messages: {} as Record<string, Record<string, string>>,
})

const localeLoader = createLocaleLoader(
    async (locale) => {
        const appPath = `./locales/${locale}/index.json`
        const [appMessages, uiMessages] = await Promise.all([
            localeModules[appPath](),
            uiLocaleModules[appPath](),
        ])
        i18n.global.setLocaleMessage(
            locale,
            buildLocaleMessages({ [appPath]: appMessages }, { [appPath]: uiMessages })[locale],
        )
    },
    (locale) => {
        i18n.global.locale.value = locale
    },
)

export async function setAppLocale(locale: string): Promise<void> {
    await localeLoader.select(availableLocales.includes(locale) ? locale : 'en-US')
}

export async function initializeLocale(): Promise<void> {
    await localeLoader.ensure('en-US')
    await applyLocalePreference(null)
}

export function resolveInitialLocale(preferredLocales: readonly string[]): string {
    for (const preferredLocale of preferredLocales) {
        const normalizedLocale = preferredLocale.replace('_', '-')
        if (availableLocales.includes(normalizedLocale)) return normalizedLocale

        const language = normalizedLocale.split('-')[0].toLowerCase()
        if (language === 'zh') {
            const traditionalChinese = /-(tw|hk|mo)|-hant/i.test(normalizedLocale)
            return traditionalChinese ? 'zh-TW' : 'zh-CN'
        }

        const languageMatch = availableLocales.find((locale) =>
            locale.toLowerCase().startsWith(`${language}-`),
        )
        if (languageMatch) return languageMatch
    }

    return 'en-US'
}

export const SYSTEM_LOCALE_VALUE = 'system'
const FOLLOW_SYSTEM_LOCALE_KEY = 'axolotl.follow-system-locale'

function preferredNavigatorLocales(): readonly string[] {
    return navigator.languages?.length ? navigator.languages : [navigator.language ?? 'en-US']
}

export function getSystemResolvedLocale(): string {
    return resolveInitialLocale(preferredNavigatorLocales())
}

export function isFollowingSystemLocale(storedLocale: string | null | undefined): boolean {
    if (!storedLocale || storedLocale === SYSTEM_LOCALE_VALUE) return true
    try {
        return localStorage.getItem(FOLLOW_SYSTEM_LOCALE_KEY) === '1'
    } catch {
        return false
    }
}

export function setFollowSystemLocale(follow: boolean) {
    try {
        if (follow) localStorage.setItem(FOLLOW_SYSTEM_LOCALE_KEY, '1')
        else localStorage.removeItem(FOLLOW_SYSTEM_LOCALE_KEY)
    } catch {
        // localStorage may be unavailable; system-follow then relies on empty/'system' locale only.
    }
}

/** Applies the stored locale preference and returns the concrete locale used. */
export async function applyLocalePreference(
    storedLocale: string | null | undefined,
): Promise<string> {
    const locale = isFollowingSystemLocale(storedLocale)
        ? getSystemResolvedLocale()
        : availableLocales.includes(storedLocale ?? '')
          ? storedLocale!
          : 'en-US'
    await setAppLocale(locale)
    return locale
}

// The locale the user picked lives in the app database, which is exactly what is
// unreadable when startup fails - and a startup failure is when the remaining
// dialogs matter most. Starting from the system language keeps them in a
// language the user reads; `setupApp` still applies the saved locale once the
// database is available.
export default i18n
