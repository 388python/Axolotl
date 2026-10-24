export type DownloadSource = 'auto' | 'update-server' | 'github'
export type ResolvedDownloadSource = Exclude<DownloadSource, 'auto'>
// auto 模式在解析上的主源保持不变（update-server），GitLink 只是主源异常时的降级源，
// 因此它是可被判定的描述，而不是一个需要用户选择的来源。
export type FallbackDownloadSource = 'gitlink'

export const fallbackSource: FallbackDownloadSource = 'gitlink'

const DOWNLOAD_SOURCE_STORAGE_KEY = 'axolotl-download-source'
function isDownloadSource(value: string | null): value is DownloadSource {
    return value === 'auto' || value === 'update-server' || value === 'github'
}

export function useDownloadSource() {
    const selectedSource = useState<DownloadSource>('axolotl-download-source', () => 'auto')

    // auto 模式保持 update-server 优先；是否真的用降级源由页面在两源都取到数据后判定。
    const resolvedSource = computed<ResolvedDownloadSource>(() => {
        if (selectedSource.value !== 'auto') return selectedSource.value
        return 'update-server'
    })

    function setDownloadSource(source: DownloadSource) {
        selectedSource.value = source
        if (import.meta.client) {
            localStorage.setItem(DOWNLOAD_SOURCE_STORAGE_KEY, source)
        }
    }

    onMounted(() => {
        const savedSource = localStorage.getItem(DOWNLOAD_SOURCE_STORAGE_KEY)
        if (isDownloadSource(savedSource)) {
            selectedSource.value = savedSource
        }
    })

    return {
        selectedSource,
        resolvedSource,
        fallbackSource,
        setDownloadSource,
    }
}
