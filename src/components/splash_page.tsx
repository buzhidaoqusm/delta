import React, { useState, useEffect } from 'react'
import { Card, CardAction, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from './ui/card'
import DiskPath from './disk_path'
import { Separator } from '@/components/ui/separator'
import CustomPath from './custom_path'
import { Button } from './ui/button'
import { Checkbox } from './ui/checkbox'
import { Label } from '@/components/ui/label'

import { invoke } from '@tauri-apps/api/core';
import { snapshotStore, useErrorStore, userStore } from './store'
import { DataTable } from './data_table'
import { SnapshotFile } from './data_table_columns'

import { createSnapshotColumns } from './data_table_columns'
import { RowSelectionState } from '@tanstack/react-table'
import Progress from './progress'

import DeltaLogo from '../../src-tauri/icons/64x64.png'
import { DirView, InitDisk } from '@/types'
import TopBar from './top-bar'
import { ScanTabs } from './ScanTabs'
import { useTranslation } from 'react-i18next'

interface SplashPageProps {
  setWhichField: React.Dispatch<React.SetStateAction<boolean>>;
}

const SplashPage: React.FC<SplashPageProps> = ({ setWhichField }) => {
  const { t, i18n } = useTranslation()
  const columns = createSnapshotColumns(t, i18n.resolvedLanguage ?? i18n.language)

  // const [disks, setDisks] = useState<InitDisk[]>([]);

  // const [selectedDisk, setSelectedDisk] = useState<string>(""); // for full disk paths

  const [rowSelection, setRowSelection] = useState<RowSelectionState>({});

  // const [saveCurrentSnapshotFlag, setSaveCurrentSnapshotFlag] = useState<boolean>(true);

  const snapshotFiles = snapshotStore((state) => state.previousSnapshots)

  const setSnapshotFiles = snapshotStore((state) => state.setPreviousSnapshots)

  // const snapshotFlag = userStore((state) => state.snapshotFlag) // this flag is for if to save snapshot or not 

  const setSnapshotFlag = userStore((state) => state.setSnapshotFlag)

  // const snapshotFile = userStore((state) => state.prevSnapshotFilePath)

  const setSnapshotFile = userStore((state) => state.setSelectedHistorySnapshotFile) // this is for the SELECTED history file string!

  const setCurrentBackendError = useErrorStore((state) => state.setCurrentBackendError)

  const snapshotFileName = (snapshot: SnapshotFile) =>
    `${snapshot.drive_letter}_${snapshot.date_sort_key}_${snapshot.size}`

  const selectedSnapshots = Object.keys(rowSelection)
    .map((index) => snapshotFiles[Number(index)])
    .filter(Boolean)

  const getCompareDisabledReason = (snapshots: SnapshotFile[]) => {
    if (snapshots.length !== 2) return t("snapshot.compareSelectTwo")
    if (!snapshots.every((snapshot) => snapshot.can_compare)) {
      return t("snapshot.compareRequiresNew")
    }

    const [first, second] = snapshots
    if (first.root_path && second.root_path && first.root_path !== second.root_path) {
      return t("snapshot.compareRequiresSameRoot")
    }
    if (!first.root_path && !second.root_path && first.drive_letter !== second.drive_letter) {
      return t("snapshot.compareRequiresSameRoot")
    }

    return ""
  }

  const compareDisabledReason = getCompareDisabledReason(selectedSnapshots)

  const openSnapshotPreview = async (snapshot: SnapshotFile) => {
    if (!snapshot.can_preview) {
      setCurrentBackendError({
        err_code: 1001,
        user_error_string_desc: t("snapshot.legacyPreviewUnsupported"),
        library_generated_error_desc: "N/A",
      })
      return
    }

    try {
      const result = await invoke<DirView>('open_snapshot_preview', {
        snapshotFileName: snapshotFileName(snapshot),
      })

      userStore.getState().initSnapshotPreviewData(result, snapshotFileName(snapshot))
      setWhichField(false)
    } catch (err) {
      setCurrentBackendError(err)
    }
  }

  const runSnapshotCompare = async () => {
    if (compareDisabledReason || selectedSnapshots.length !== 2) return

    try {
      const [first, second] = selectedSnapshots
      const result = await invoke<DirView>('compare_snapshots', {
        firstSnapshotFileName: snapshotFileName(first),
        secondSnapshotFileName: snapshotFileName(second),
      })

      const ordered = [first, second].sort((a, b) => b.date_sort_key - a.date_sort_key)
      userStore.getState().initSnapshotCompareData(
        result,
        snapshotFileName(ordered[0]),
        snapshotFileName(ordered[1])
      )
      setWhichField(false)
    } catch (err) {
      setCurrentBackendError(err)
    }
  }


  useEffect(() => {

    const getSnapshotTable = async () => {
      try {
        const resp: SnapshotFile[] = await invoke('get_local_snapshot_files')
        setSnapshotFiles(resp)
      } catch (err) {
        setCurrentBackendError(err)
      }
    }

    getSnapshotTable()
  }, []);

  // for bridging selected snapshot file and zustand global state for it
  useEffect(() => {
    const selectedIndexes = Object.keys(rowSelection)

    if (selectedIndexes.length !== 1) {
      setSnapshotFile("") // reset these on mount
      setSnapshotFlag(false)
      return;
    }

    const selectedData = snapshotFiles[parseInt(selectedIndexes[0])]

    if (!selectedData) {
      setSnapshotFile("")
      setSnapshotFlag(false)
      return;
    }

    setSnapshotFile(snapshotFileName(selectedData)) // sync to zustand
  }, [rowSelection, snapshotFiles, setSnapshotFile]);


  return (
    <div className="flex flex-col h-screen bg-stone-800">
      <TopBar></TopBar>

      <div className="flex flex-1 flex-wrap items-center justify-center gap-6 p-6 overflow-auto">

        {/* Temp image */}
        <img src={DeltaLogo} alt={t("app.logoAlt")} className='transition-all duration-500 hover:scale-150 hover:rotate-180 opacity-90 hover:opacity-100 cursor-pointer fixed bottom-9 right-9' />

        {/* Test data table for snapshots, datatable should be generic */}
        <Card className='p-3 min-w-[350px] flex flex-col gap-3'>
          <DataTable
            columns={columns}
            data={snapshotFiles}
            rowSelection={rowSelection}
            setRowSelection={setRowSelection}
            onRowDoubleClick={openSnapshotPreview}
            maxSelectedRows={2}
          ></DataTable>
          <div className="flex flex-col gap-1">
            <Button
              variant="outline"
              disabled={Boolean(compareDisabledReason)}
              onClick={runSnapshotCompare}
            >
              {t("snapshot.compareSnapshots")}
            </Button>
            {compareDisabledReason && (
              <p className="text-xs text-muted-foreground text-center">
                {compareDisabledReason}
              </p>
            )}
          </div>
        </Card>

        {/* disk scan tabs */}
        <ScanTabs setWhichField={setWhichField}></ScanTabs>

      </div>
    </div>
  )
}

export default SplashPage
