import React, { useEffect, useState } from 'react'
import SplashPage from '@/components/splash_page'
import { ThemeProvider } from './components/theme-provider'
import Analytics from './components/analytics'
import { listen } from '@tauri-apps/api/event'
import { LiveScanEntryEvent, LiveScanFileBatchEvent } from './types'
import { userStore } from './components/store'

const App = () => {

  const [whichField, setWhichField] = useState<boolean>(true);

  useEffect(() => {
    const unlistenEntry = listen<LiveScanEntryEvent>('live-scan-entry', (event) => {
      userStore.getState().applyLiveScanEntry(event.payload);
    });
    const unlistenFileBatch = listen<LiveScanFileBatchEvent>('live-scan-file-batch', (event) => {
      userStore.getState().applyLiveScanFileBatch(event.payload);
    });

    return () => {
      unlistenEntry.then((dispose) => dispose());
      unlistenFileBatch.then((dispose) => dispose());
    };
  }, []);

  return (
    <ThemeProvider defaultTheme="dark" storageKey="vite-ui-theme">

      {whichField ? (
        <SplashPage setWhichField={setWhichField}></SplashPage>
      ) : (
        <Analytics setWhichField={setWhichField}></Analytics>
      )}
    </ThemeProvider>
  )
}

export default App
