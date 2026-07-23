import { Checkbox } from '@charcoal-ui/react'
import { useAppContext } from '../context/AppContext'

export function MicDrainerPanel() {
  const { t, micDrainerEnabled, micDrainerStatus, micDrainerPeak, toggleMicDrainer } =
    useAppContext()

  // ステータスに応じたドット色
  const dotClass =
    micDrainerStatus === 'draining'
      ? 'bg-brand'
      : micDrainerStatus === 'waiting'
        ? 'bg-amber-400'
        : 'bg-gray-300 dark:bg-gray-600'

  const statusText =
    micDrainerStatus === 'draining'
      ? t('mic_drainer_status_draining')
      : micDrainerStatus === 'waiting'
        ? t('mic_drainer_status_waiting')
        : t('mic_drainer_status_off')

  // レベルメーター幅（0–100%）。off のときは 0。
  const meterPct = micDrainerEnabled ? Math.min(100, Math.round(micDrainerPeak * 100)) : 0

  return (
    <div className='bg-white dark:bg-gray-800 rounded-lg p-5 border border-gray-200 dark:border-gray-700 w-full max-w-sm'>
      {/* ヘッダー: タイトル + ステータス */}
      <div className='flex justify-between items-center mb-3'>
        <span className='font-semibold text-sm text-gray-800 dark:text-gray-200'>
          {t('mic_drainer_title')}
        </span>
        <div className='flex items-center gap-1.5'>
          <span className={`relative inline-flex rounded-full h-2 w-2 ${dotClass}`}>
            {micDrainerStatus === 'draining' && (
              <span className='animate-ping absolute inline-flex h-full w-full rounded-full bg-brand opacity-60' />
            )}
          </span>
          <span className='text-xs text-gray-500 dark:text-gray-400'>{statusText}</span>
        </div>
      </div>

      {/* レベルメーター */}
      <div className='h-2 w-full rounded-full bg-gray-100 dark:bg-gray-700 overflow-hidden mb-3'>
        <div
          className='h-full rounded-full bg-brand transition-[width] duration-100 ease-out'
          style={{ width: `${meterPct}%` }}
        />
      </div>

      {/* トグル */}
      <div className='text-sm text-gray-900 dark:text-gray-100'>
        <Checkbox checked={micDrainerEnabled} onChange={toggleMicDrainer}>
          {t('mic_drainer_toggle')}
        </Checkbox>
      </div>

      {/* 補足 */}
      <p className='mt-2 text-xs text-gray-400 dark:text-gray-500'>{t('mic_drainer_note')}</p>
    </div>
  )
}
