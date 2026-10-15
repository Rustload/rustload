<script lang="ts">
  import ChartColumn from '@lucide/svelte/icons/chart-column'
  import { api, type Stats } from '$lib/api'
  import BarList from '$lib/components/stats/BarList.svelte'
  import ColumnChart, { type Column } from '$lib/components/stats/ColumnChart.svelte'
  import Alert from '$lib/components/ui/Alert.svelte'
  import EmptyState from '$lib/components/ui/EmptyState.svelte'
  import Loading from '$lib/components/ui/Loading.svelte'
  import PageHeader from '$lib/components/ui/PageHeader.svelte'
  import Segmented from '$lib/components/ui/Segmented.svelte'
  import { t } from '$lib/i18n/index.svelte'
  import { accounts } from '$lib/stores/catalog.svelte'
  import { errorMessage } from '$lib/stores/toasts.svelte'
  import {
    formatBytes,
    formatCalendar,
    formatDate,
    formatNumber,
    formatSpeed,
    niceBytesCeil,
    splitBytes,
  } from '$lib/utils/format'

  type Range = '7' | '30' | '90' | '365'

  let range = $state<Range>('30')
  let data = $state.raw<Stats | null>(null)
  let error = $state<string | null>(null)
  let loading = $state(false)
  let showTable = $state(false)
  let request = 0

  $effect(() => {
    const days = Number(range)
    const id = ++request
    loading = true
    error = null
    api.stats
      .get(days)
      .then((stats) => {
        if (id === request) data = stats
      })
      .catch((err) => {
        if (id === request) error = errorMessage(err)
      })
      .finally(() => {
        if (id === request) loading = false
      })
  })

  const total = $derived(data ? splitBytes(data.totals.bytes) : null)

  function parseDay(day: string): Date {
    const [y, m, d] = day.split('-').map(Number)
    return new Date(y, m - 1, d)
  }

  function details(files: number, failed: number): string[] {
    const lines = [t('stats.filesCount', { count: files })]
    if (failed) lines.push(t('stats.failedCount', { count: failed }))
    return lines
  }

  /** Days up to a month, weeks up to a quarter, months beyond. */
  const columns = $derived.by((): Column[] => {
    if (!data) return []
    const days = data.daily
    if (days.length <= 31) {
      return days.map((d, i) => ({
        label: formatCalendar(parseDay(d.day), 'day'),
        title: formatCalendar(parseDay(d.day), 'longDay'),
        value: d.bytes,
        details: details(d.files, d.failed),
        live: i === days.length - 1,
      }))
    }
    const buckets = new Map<string, { start: Date; bytes: number; files: number; failed: number }>()
    for (const d of days) {
      const date = parseDay(d.day)
      const start =
        days.length <= 120
          ? new Date(date.getFullYear(), date.getMonth(), date.getDate() - ((date.getDay() + 6) % 7))
          : new Date(date.getFullYear(), date.getMonth(), 1)
      const key = start.toDateString()
      const bucket = buckets.get(key) ?? { start, bytes: 0, files: 0, failed: 0 }
      bucket.bytes += d.bytes
      bucket.files += d.files
      bucket.failed += d.failed
      buckets.set(key, bucket)
    }
    const list = [...buckets.values()]
    const weekly = days.length <= 120
    return list.map((b, i) => ({
      label: formatCalendar(b.start, weekly ? 'day' : 'month'),
      title: weekly
        ? t('stats.weekOf', { date: formatCalendar(b.start, 'day') })
        : formatCalendar(b.start, 'longMonth'),
      value: b.bytes,
      details: details(b.files, b.failed),
      live: i === list.length - 1,
    }))
  })

  const hourColumns = $derived(
    (data?.hours ?? []).map((bytes, hour) => ({
      label: `${hour}`,
      title: t('stats.hourRange', { from: hour, to: (hour + 1) % 24 }),
      value: bytes,
      details: [],
    })),
  )

  const hostRows = $derived(
    (data?.hosts ?? []).map((h) => ({
      key: h.host,
      label: h.host === 'other' ? t('stats.otherHosts') : h.host || t('stats.unknownHost'),
      value: h.bytes,
      detail: t('stats.filesCount', { count: h.files }),
    })),
  )

  const providerRows = $derived(
    (data?.providers ?? []).map((p) => ({
      key: p.via,
      label: p.via === 'direct' ? t('stats.direct') : accounts.providerName(p.via),
      value: p.bytes,
      detail: t('stats.filesCount', { count: p.files }),
    })),
  )

  const empty = $derived(data !== null && data.all_time.files === 0)
  const bytes = (v: number) => formatBytes(v, { trim: true })
</script>

<PageHeader title={t('stats.title')}>
  {#snippet actions()}
    <Segmented
      label={t('stats.range')}
      bind:value={range}
      options={[
        { value: '7', label: t('stats.ranges.7') },
        { value: '30', label: t('stats.ranges.30') },
        { value: '90', label: t('stats.ranges.90') },
        { value: '365', label: t('stats.ranges.365') },
      ]}
    />
  {/snippet}
</PageHeader>

{#if error}
  <Alert>{error}</Alert>
{:else if !data}
  <Loading />
{:else if empty}
  <EmptyState icon={ChartColumn} title={t('stats.emptyTitle')} body={t('stats.emptyBody')} />
{:else}
  <div class="space-y-10 transition-opacity {loading ? 'opacity-60' : ''}">
    <section aria-labelledby="stats-volume" class="border-b border-line pb-8">
      <div class="grid gap-x-10 gap-y-6 lg:grid-cols-[minmax(14rem,auto)_minmax(0,1fr)]">
        <div>
          <h2 id="stats-volume" class="sr-only">{t('stats.volume')}</h2>
          <p class="flex items-baseline gap-2">
            <span class="readout text-[4.5rem] leading-[0.9] font-[350]">{total?.value}</span>
            <span class="text-lg font-medium text-fg-muted">{total?.unit}</span>
          </p>
          <p class="mt-2 text-fg-muted">{t(`stats.downloadedIn.${range}`)}</p>

          <dl class="mt-6 grid grid-cols-2 gap-x-6 gap-y-4">
            <div>
              <dt class="text-small text-fg-muted">{t('stats.files')}</dt>
              <dd class="num text-2xl">{formatNumber(data.totals.files)}</dd>
            </div>
            <div>
              <dt class="text-small text-fg-muted">{t('stats.packages')}</dt>
              <dd class="num text-2xl">{formatNumber(data.totals.packages)}</dd>
            </div>
            <div>
              <dt class="text-small text-fg-muted">{t('stats.avgSpeed')}</dt>
              <dd class="num text-2xl">
                {data.totals.avg_speed ? formatSpeed(data.totals.avg_speed) : '—'}
              </dd>
            </div>
            <div>
              <dt class="text-small text-fg-muted">{t('stats.peakSpeed')}</dt>
              <dd class="num text-2xl">
                {data.totals.peak_speed ? formatSpeed(data.totals.peak_speed) : '—'}
              </dd>
            </div>
            {#if data.totals.failed}
              <div>
                <dt class="text-small text-fg-muted">{t('stats.failed')}</dt>
                <dd class="num text-2xl text-bad">{formatNumber(data.totals.failed)}</dd>
              </div>
            {/if}
          </dl>
        </div>

        <figure class="min-w-0 self-end">
          <ColumnChart
            {columns}
            format={bytes}
            niceMax={niceBytesCeil}
            label={t('stats.volumeChart')}
            height={220}
            labelEvery={columns.length > 14 ? 7 : 1}
          />
          <figcaption class="mt-3 flex flex-wrap items-center justify-between gap-2 text-xs text-fg-faint">
            <span>{t(columns.length > 31 || Number(range) > 31 ? 'stats.captionGrouped' : 'stats.captionDaily')}</span>
            <button
              type="button"
              class="underline underline-offset-2 hover:text-fg"
              aria-expanded={showTable}
              onclick={() => (showTable = !showTable)}
            >
              {showTable ? t('stats.hideTable') : t('stats.showTable')}
            </button>
          </figcaption>
        </figure>
        {#if showTable}
          <div class="max-h-72 overflow-y-auto rounded-control border border-line lg:col-span-2">
            <table class="w-full text-small">
              <thead class="sticky top-0 bg-panel text-left text-fg-muted">
                <tr>
                  <th class="px-3 py-2 font-medium">{t('stats.period')}</th>
                  <th class="px-3 py-2 text-right font-medium">{t('stats.volume')}</th>
                  <th class="px-3 py-2 text-right font-medium">{t('stats.files')}</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-line">
                {#each columns as column, i (i)}
                  <tr>
                    <td class="px-3 py-1.5">{column.title ?? column.label}</td>
                    <td class="px-3 py-1.5 text-right num">{formatBytes(column.value)}</td>
                    <td class="px-3 py-1.5 text-right num text-fg-muted">{column.details[0]}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>
    </section>

    <div class="grid gap-x-12 gap-y-10 lg:grid-cols-3">
      <section aria-labelledby="stats-hours">
        <h2 id="stats-hours" class="heading font-semibold">{t('stats.hours')}</h2>
        <p class="mt-0.5 mb-4 text-small text-fg-muted">{t('stats.hoursHint')}</p>
        <ColumnChart
          columns={hourColumns}
          format={bytes}
          niceMax={niceBytesCeil}
          label={t('stats.hours')}
          height={150}
          labelEvery={6}
        />
        {#if showTable}
          <table class="mt-4 w-full text-small">
            <thead class="text-left text-fg-muted">
              <tr>
                <th class="py-1 font-medium">{t('stats.hour')}</th>
                <th class="py-1 text-right font-medium">{t('stats.volume')}</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-line">
              {#each hourColumns as column (column.label)}
                <tr>
                  <td class="py-1">{column.title}</td>
                  <td class="py-1 text-right num">{formatBytes(column.value)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </section>

      <section aria-labelledby="stats-hosts">
        <h2 id="stats-hosts" class="heading font-semibold">{t('stats.hosts')}</h2>
        <p class="mt-0.5 mb-4 text-small text-fg-muted">{t('stats.hostsHint')}</p>
        <BarList rows={hostRows} format={bytes} label={t('stats.hosts')} />
      </section>

      <section aria-labelledby="stats-providers">
        <h2 id="stats-providers" class="heading font-semibold">{t('stats.providers')}</h2>
        <p class="mt-0.5 mb-4 text-small text-fg-muted">{t('stats.providersHint')}</p>
        <BarList rows={providerRows} format={bytes} label={t('stats.providers')} />
      </section>
    </div>

    {#if data.all_time.since}
      <p class="border-t border-line pt-5 text-small text-fg-muted">
        {t('stats.allTime', {
          date: formatDate(data.all_time.since),
          size: formatBytes(data.all_time.bytes),
          count: data.all_time.files,
        })}
      </p>
    {/if}
  </div>
{/if}
