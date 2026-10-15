<script lang="ts">
  import type { DownloadStatus, PackageStatus } from '$lib/api'
  import { t } from '$lib/i18n/index.svelte'
  import { downloadTone, packageTone } from '$lib/utils/status'
  import Badge from './Badge.svelte'

  type Props =
    | { kind: 'package'; status: PackageStatus; class?: string }
    | { kind: 'download'; status: DownloadStatus; class?: string }

  let props: Props = $props()

  const tone = $derived(props.kind === 'package' ? packageTone[props.status] : downloadTone[props.status])
  const label = $derived(
    props.kind === 'package' ? t(`status.package.${props.status}`) : t(`status.download.${props.status}`),
  )
  const live = $derived(props.status === 'downloading' || props.status === 'extracting' || props.status === 'debrid')
</script>

<Badge {label} {tone} dot pulse={live} class={props.class} />
