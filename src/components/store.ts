import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { AnalysisMode, BackendError, CurrentEntryDetails, DirView, DirViewChildren, LiveScanEntryEvent, LiveScanFileBatchEvent, TreeDataNode } from "@/types";
import { appendPaths } from "@/lib/utils";
import { SnapshotFile } from "./data_table_columns";

// caching ht for history graph making it a singleton for now
let historyCache: Record<string, { timestamp: number; sizeBytes: number }[]> = {}
// TODO cache clear helper
export const clearHistoryCache = () => {
  historyCache = {};
};

// To get the path we could traverse up the tree or we could store it as a field in the interface
// the root is the global state, everything else is helper functions
interface FrontEndFileSystemStore {
  root: TreeDataNode;
  analysisMode: AnalysisMode;
  liveScanStatus: "idle" | "scanning" | "complete" | "error";
  liveScanEntryCount: number;
  activeSnapshotFile: string;
  newerSnapshotFile: string;
  olderSnapshotFile: string;
  currentPath: string; // used for the temporary onhover path thing
  currentEntryDetail: CurrentEntryDetails; // used for the quick detail at top bar
  currentEntryData: TreeDataNode; // used for the side overview
  snapshotFlag: boolean; // a frontend state flag that represents if requests are for snapshot comparing or not true = compare false = don't compare
  prevSnapshotFilePath: string;
  addNewDirView: (currentTreeData: TreeDataNode, pathList: string[]) => void;
  changeCurrentOverviewNode: (currentTreeNode: TreeDataNode) => void;
  changeCurrentPath: (path: string) => void;
  changeCurrentEntryDetails: (numsubdir: number, numsubfile: number) => void;
  beginLiveScan: (rootPath: string) => void;
  applyLiveScanEntry: (entry: LiveScanEntryEvent) => void;
  applyLiveScanFileBatch: (batch: LiveScanFileBatchEvent) => void;
  finishLiveScan: (inital: DirView, rootPath: string) => void;
  failLiveScan: () => void;
  initDirData: (inital: DirView, rootPath: string) => void;
  initSnapshotPreviewData: (initial: DirView, snapshotFile: string) => void;
  initSnapshotCompareData: (initial: DirView, newerSnapshotFile: string, olderSnapshotFile: string) => void;
  setAnalysisMode: (mode: AnalysisMode) => void;
  setActiveSnapshotFile: (snapshotFile: string) => void;
  setSnapshotFlag: (flag: boolean) => void;
  setSelectedHistorySnapshotFile: (file: string) => void;
}

interface FrontEndSnapshotStore {
  previousSnapshots: SnapshotFile[];
  setPreviousSnapshots: (snapshotFileList: SnapshotFile[]) => void; // need spread to force rerender
}

interface ErrorStore {
  currentBackendErrors: BackendError[];
  setCurrentBackendError: (newError: BackendError) => void; // send current backend error based on a new 
}

interface DirEntryHistoryStore {
  currentDirEntryHistory: { timestamp: number; sizeBytes: number }[];
  activeHistoryPath: string | null; // rc condition helper
  queryDirEntryHistory: (rootPath: string, absolutePath: string) => void;
  setCurrentDirEntryHistory: (newHistory: { timestamp: number; sizeBytes: number }[]) => void;
}

interface FrontEndConfigurationStore {
  ShowHistory: boolean;
  setShowHistory: (flag: boolean) => void;
  // If ever need more configs add here
  // saving configs to persistent config file in future also 
}

export const useConfigurationStore = create<FrontEndConfigurationStore>((set) => ({
  ShowHistory: false,
  setShowHistory: (flag) => {
    set({ ShowHistory: flag })
  }
}))

export const useDirEntryHistoryStore = create<DirEntryHistoryStore>((set, get) => ({
  currentDirEntryHistory: [],
  activeHistoryPath: null,

  queryDirEntryHistory: async (rootPath, absolutePath) => {

    // mark curr as the current active, this line is sync with onClick call so order is ensured
    set({ activeHistoryPath: absolutePath });

    // Check if history flag disabled
    const showHistoryFlag = useConfigurationStore.getState().ShowHistory;

    if (!showHistoryFlag) {
      return;
    }

    if (historyCache[absolutePath]) {
      set({ currentDirEntryHistory: historyCache[absolutePath] });
      return;
    }

    try {
      const result: [string, number][] = await invoke(
        'get_path_historical_data',
        { rootPath, absolutePath }
      );

      console.log(result)

      // If many async calls to this func is scheduled in rt then no guarentee of the order
      // so only let the correct name one change state using a fast helper to mark the activeHistoryPath
      if (get().activeHistoryPath !== absolutePath) {
        return;
      }

      const formattedHistory = result.map(([dateStr, sizeBytes]) => ({
        timestamp: new Date(dateStr).getTime(),
        sizeBytes,
      }));

      set({ currentDirEntryHistory: formattedHistory });
      historyCache[absolutePath] = formattedHistory;
    } catch (error) {
      // for error set curr to empty
      useErrorStore.getState().setCurrentBackendError(error as BackendError);
      set({ currentDirEntryHistory: [] });
    }
  },

  setCurrentDirEntryHistory: (newHistory) => set({ currentDirEntryHistory: newHistory }),
}));


export const useErrorStore = create<ErrorStore>((set) => ({
  currentBackendErrors: [],
  setCurrentBackendError: (newError) => set((state) => ({ // append new error to list and forces ref change
    currentBackendErrors: [...state.currentBackendErrors, newError]
  })),
}));


export const snapshotStore = create<FrontEndSnapshotStore>((set, get) => ({
  previousSnapshots: [],
  setPreviousSnapshots: (snapshotFileList) => {
    set({ previousSnapshots: snapshotFileList })
  },
}))

const mapDirViewChildrenToTreeNodes = (
  result: DirViewChildren,
  currentNode: TreeDataNode
): TreeDataNode[] => {
  const subdirs = result.subdirviews.map((subdir) => ({
    id: subdir.id,
    name: subdir.name,
    size: subdir.meta.size,
    numsubdir: subdir.meta.num_subdir,
    numsubfiles: subdir.meta.num_files,

    diff: subdir.meta.diff ? {
      new_flag: subdir.meta.diff.new_dir_flag,
      deleted_flag: subdir.meta.diff.deleted_dir_flag,
      prevnumsubdir: subdir.meta.diff.prev_num_subdir,
      prevnumfiles: subdir.meta.diff.prev_num_files,
      prevsize: subdir.meta.diff.previous_size,
    } : undefined,

    created: new Date(subdir.meta.created.secs_since_epoch * 1000),
    modified: new Date(subdir.meta.modified.secs_since_epoch * 1000),
    path: subdir.path ?? appendPaths(currentNode.path, subdir.name),
    children: [],
    childrenLoaded: false,
    scanDiscovered: true,
    directory: true,
  }));

  const files = result.files.map((file) => ({
    id: file.id,
    name: file.name,
    size: file.meta.size,
    path: file.path ?? appendPaths(currentNode.path, file.name),

    diff: file.meta.diff ? {
      new_flag: file.meta.diff.new_file_flag,
      prevsize: file.meta.diff.previous_size,
      deleted_flag: file.meta.diff.deleted_file_flag,
    } : undefined,
    directory: false,

    created: new Date(file.meta.created.secs_since_epoch * 1000),
    modified: new Date(file.meta.modified.secs_since_epoch * 1000),
  }));

  return [...subdirs, ...files];
};

const timeFromBackend = (time: { secs_since_epoch: number }) =>
  new Date(time.secs_since_epoch * 1000);

const makeLiveRootNode = (rootPath: string): TreeDataNode => ({
  id: `live:${rootPath}`,
  name: rootPath,
  path: rootPath,
  children: [],
  childrenLoaded: false,
  scanDiscovered: true,
  size: 0,
  numsubdir: 0,
  numsubfiles: 0,
  directory: true,
});

const makeLivePlaceholderDir = (path: string, name: string): TreeDataNode => ({
  id: `live:${path}`,
  name,
  path,
  children: [],
  childrenLoaded: false,
  scanDiscovered: false,
  size: 0,
  numsubdir: 0,
  numsubfiles: 0,
  directory: true,
});

const findNodeByPath = (node: TreeDataNode, path?: string): TreeDataNode | undefined => {
  if (!path) return undefined;
  if (node.path === path) return node;

  for (const child of node.children ?? []) {
    const found = findNodeByPath(child, path);
    if (found) return found;
  }

  return undefined;
};

const getPathSegmentsFromRoot = (rootPath: string, fullPath: string): string[] => {
  if (!fullPath || fullPath === rootPath || !fullPath.startsWith(rootPath)) {
    return [];
  }

  return fullPath
    .slice(rootPath.length)
    .split(/[\\/]+/)
    .filter(Boolean);
};

const ensureDirectoryPath = (root: TreeDataNode, targetPath: string): TreeDataNode => {
  if (!targetPath || targetPath === root.path) {
    return root;
  }

  const rootPath = root.path ?? "";
  const segments = getPathSegmentsFromRoot(rootPath, targetPath);
  let current = root;
  let currentPath = rootPath;

  for (const segment of segments) {
    currentPath = appendPaths(currentPath, segment);
    const children = current.children ?? [];
    let next = children.find((child) => child.directory && child.path === currentPath);

    if (!next) {
      next = makeLivePlaceholderDir(currentPath, segment);
      current.children = [...children, next];
    }

    current = next;
  }

  return current;
};

const getDirectoryPathChain = (root: TreeDataNode, targetPath: string): TreeDataNode[] => {
  const chain = [root];
  const rootPath = root.path ?? "";
  const segments = getPathSegmentsFromRoot(rootPath, targetPath);
  let current = root;
  let currentPath = rootPath;

  for (const segment of segments) {
    currentPath = appendPaths(currentPath, segment);
    current = ensureDirectoryPath(current, currentPath);
    chain.push(current);
  }

  return chain;
};

const cloneForCurrentEntry = (node: TreeDataNode): TreeDataNode => ({
  ...node,
  children: node.children,
});

const applyLiveScanEntryMutation = (root: TreeDataNode, entry: LiveScanEntryEvent): boolean => {
  if (!root.path || !entry.path.startsWith(root.path)) {
    return false;
  }

  const node = ensureDirectoryPath(root, entry.path);
  const wasDiscovered = node.scanDiscovered === true;
  const oldSize = node.size ?? 0;
  const nextSize = entry.size ?? 0;

  if (!wasDiscovered && entry.path !== root.path && entry.parent_path) {
    const parent = ensureDirectoryPath(root, entry.parent_path);
    parent.numsubdir = (parent.numsubdir ?? 0) + 1;
  }

  node.id = entry.id;
  node.name = entry.name;
  node.size = nextSize;
  node.numsubfiles = entry.num_files;
  node.numsubdir = entry.num_subdir;
  node.created = timeFromBackend(entry.created);
  node.modified = timeFromBackend(entry.modified);
  node.scanDiscovered = true;
  node.childrenLoaded = false;

  const delta = nextSize - oldSize;
  if (delta !== 0 && entry.path !== root.path && entry.parent_path) {
    const parentChain = getDirectoryPathChain(root, entry.parent_path);
    parentChain.forEach((ancestor) => {
      ancestor.size = (ancestor.size ?? 0) + delta;
    });
  }

  return true;
};

const applyLiveScanFileBatchMutation = (
  root: TreeDataNode,
  batch: LiveScanFileBatchEvent
): number => {
  let appliedCount = 0;

  for (const update of batch.updates) {
    if (!root.path || !update.parent_path.startsWith(root.path)) {
      continue;
    }

    const parent = ensureDirectoryPath(root, update.parent_path);
    const chain = getDirectoryPathChain(root, update.parent_path);

    chain.forEach((node) => {
      node.size = (node.size ?? 0) + update.size;
    });
    parent.numsubfiles = (parent.numsubfiles ?? 0) + update.file_count;
    appliedCount += update.file_count;
  }

  return appliedCount;
};

export const userStore = create<FrontEndFileSystemStore>((set, get) => ({
  root:
  {
    id: "root",
    name: "Root",
    path: "/",
    children: [],
    childrenLoaded: false,
    size: 0,
    directory: true,
  },

  analysisMode: "live-scan",

  liveScanStatus: "idle",

  liveScanEntryCount: 0,

  activeSnapshotFile: "",

  newerSnapshotFile: "",

  olderSnapshotFile: "",

  currentEntryData:
  {
    id: "root",
    name: "Root",
    path: "/",
    children: [],
    childrenLoaded: false,
    size: 0,
    directory: true,
  },

  currentPath: "N/A",

  snapshotFlag: false, // default to do not compare snapshots

  prevSnapshotFilePath: "", // temp name for when there is nothing set and nothing chosen will be empty str

  currentEntryDetail: {
    numsubdir: 0,
    numsubfile: 0,
  },

  addNewDirView: async (currentNode, pathList) => {
    try {

      const { snapshotFlag } = get();
      const { prevSnapshotFilePath } = get();
      const { analysisMode } = get();

      let result: DirViewChildren;

      if (analysisMode === "snapshot-preview") {
        result = await invoke<DirViewChildren>(
          'query_snapshot_dir_object',
          { snapshotFileName: get().activeSnapshotFile, parentId: currentNode.id }
        );
      } else if (analysisMode === "snapshot-compare") {
        result = await invoke<DirViewChildren>(
          'query_snapshot_compare_dir_object',
          {
            newerSnapshotFileName: get().newerSnapshotFile,
            olderSnapshotFileName: get().olderSnapshotFile,
            parentId: currentNode.id,
          }
        );
      } else {
        result = await invoke<DirViewChildren>(
          'query_new_dir_object',
          { pathList, snapshotFlag, prevSnapshotFilePath }
        );
      }

      userStore.setState((state) => {
        const newChildren = mapDirViewChildrenToTreeNodes(result, currentNode);

        // Mutate the node reference directly
        currentNode.children = newChildren;
        currentNode.childrenLoaded = true;

        // FORCE RE-RENDER:
        return {
          root: { ...state.root },
        };
      });

    } catch (error) {
      useErrorStore.getState().setCurrentBackendError(error); // getState for non react lifecycle bound
      console.error(error);
      userStore.setState((state) => {
        currentNode.children = [];
        return {
          root: { ...state.root }
        };
      });
    }
  },

  changeCurrentPath: (path) =>
    set({ currentPath: path }),

  changeCurrentEntryDetails: (numsubdir, numsubfile) =>
    set({
      currentEntryDetail: { numsubdir, numsubfile },
    }),

  changeCurrentOverviewNode: (currentTreeNode) =>
    set({ currentEntryData: currentTreeNode }),

  beginLiveScan: (rootPath) => {
    const liveRoot = makeLiveRootNode(rootPath);
    set({
      analysisMode: "live-scan",
      liveScanStatus: "scanning",
      liveScanEntryCount: 0,
      activeSnapshotFile: "",
      newerSnapshotFile: "",
      olderSnapshotFile: "",
      root: liveRoot,
      currentEntryData: liveRoot,
      currentPath: rootPath,
      currentEntryDetail: {
        numsubdir: 0,
        numsubfile: 0,
      },
    });
  },

  applyLiveScanEntry: (entry) => {
    userStore.setState((state) => {
      if (state.liveScanStatus !== "scanning") {
        return state;
      }

      const root = state.root;
      if (!applyLiveScanEntryMutation(root, entry)) {
        return state;
      }

      const activeNode = findNodeByPath(root, state.currentEntryData.path) ?? root;

      return {
        root: { ...root },
        currentEntryData: cloneForCurrentEntry(activeNode),
        liveScanEntryCount: state.liveScanEntryCount + 1,
        currentEntryDetail: {
          numsubdir: activeNode.numsubdir ?? 0,
          numsubfile: activeNode.numsubfiles ?? 0,
        },
      };
    });
  },

  applyLiveScanFileBatch: (batch) => {
    if (batch.updates.length === 0) return;

    userStore.setState((state) => {
      if (state.liveScanStatus !== "scanning") {
        return state;
      }

      const root = state.root;
      const appliedCount = applyLiveScanFileBatchMutation(root, batch);

      if (appliedCount === 0) {
        return state;
      }

      const activeNode = findNodeByPath(root, state.currentEntryData.path) ?? root;

      return {
        root: { ...root },
        currentEntryData: cloneForCurrentEntry(activeNode),
        liveScanEntryCount: state.liveScanEntryCount + appliedCount,
        currentEntryDetail: {
          numsubdir: activeNode.numsubdir ?? 0,
          numsubfile: activeNode.numsubfiles ?? 0,
        },
      };
    });
  },

  finishLiveScan: (initial, rootPath) => {
    userStore.setState((state) => {
      const root = state.root.path === rootPath ? state.root : makeLiveRootNode(rootPath);

      root.id = initial.id;
      root.name = initial.name;
      root.size = initial.meta.size;
      root.numsubdir = initial.meta.num_subdir;
      root.numsubfiles = initial.meta.num_files;
      root.created = timeFromBackend(initial.meta.created);
      root.modified = timeFromBackend(initial.meta.modified);
      root.scanDiscovered = true;
      root.childrenLoaded = false;
      root.diff = initial.meta.diff ? {
        new_flag: initial.meta.diff.new_dir_flag,
        deleted_flag: initial.meta.diff.deleted_dir_flag,
        prevnumsubdir: initial.meta.diff.prev_num_subdir,
        prevnumfiles: initial.meta.diff.prev_num_files,
        prevsize: initial.meta.diff.previous_size,
      } : undefined;

      const activeNode = findNodeByPath(root, state.currentEntryData.path) ?? root;

      return {
        analysisMode: "live-scan",
        liveScanStatus: "complete",
        activeSnapshotFile: "",
        newerSnapshotFile: "",
        olderSnapshotFile: "",
        root: { ...root },
        currentEntryData: cloneForCurrentEntry(activeNode),
        currentPath: activeNode.path ?? rootPath,
        currentEntryDetail: {
          numsubdir: activeNode.numsubdir ?? 0,
          numsubfile: activeNode.numsubfiles ?? 0,
        },
      };
    });
  },

  failLiveScan: () => set({ liveScanStatus: "error" }),

  setAnalysisMode: (mode) =>
    set({ analysisMode: mode }),

  setActiveSnapshotFile: (snapshotFile) =>
    set({ activeSnapshotFile: snapshotFile }),

  initDirData: (initial, rootPath) => {
    // takes in initial dir view which is unexpanded X:\        
    // change the root based on the passed in stuff

    userStore.setState((state) => {

      const initRoot = {
        id: initial.id,
        name: initial.name,
        size: initial.meta.size,
        // path: initial.name,
        path: rootPath,
        numsubdir: initial.meta.num_subdir,
        numsubfiles: initial.meta.num_files,
        children: [],
        childrenLoaded: false,
        scanDiscovered: true,
        // sql stuff
        diff: initial.meta.diff ? {
          new_flag: initial.meta.diff.new_dir_flag,
          deleted_flag: initial.meta.diff.deleted_dir_flag,
          prevnumsubdir: initial.meta.diff.prev_num_subdir,
          prevnumfiles: initial.meta.diff.prev_num_files,
          prevsize: initial.meta.diff.previous_size,
        } : undefined,
        directory: true, // root shoudl always be a folder
      };

      return { // init current states
        analysisMode: "live-scan",
        liveScanStatus: "complete",
        activeSnapshotFile: "",
        newerSnapshotFile: "",
        olderSnapshotFile: "",
        root: initRoot,
        currentEntryData: initRoot,
        currentPath: initial.name,
      };
    }
    )
  },

  initSnapshotPreviewData: (initial, snapshotFile) => {
    userStore.setState((state) => {
      const initRoot = {
        id: initial.id,
        name: initial.name,
        size: initial.meta.size,
        path: initial.path ?? initial.name,
        numsubdir: initial.meta.num_subdir,
        numsubfiles: initial.meta.num_files,
        children: [],
        childrenLoaded: false,
        scanDiscovered: true,
        diff: initial.meta.diff ? {
          new_flag: initial.meta.diff.new_dir_flag,
          deleted_flag: initial.meta.diff.deleted_dir_flag,
          prevnumsubdir: initial.meta.diff.prev_num_subdir,
          prevnumfiles: initial.meta.diff.prev_num_files,
          prevsize: initial.meta.diff.previous_size,
        } : undefined,
        directory: true,
        sourceSnapshotFile: snapshotFile,
      };

      return {
        analysisMode: "snapshot-preview",
        liveScanStatus: "idle",
        activeSnapshotFile: snapshotFile,
        newerSnapshotFile: "",
        olderSnapshotFile: "",
        root: initRoot,
        currentEntryData: initRoot,
        currentPath: initRoot.path,
      };
    });
  },

  initSnapshotCompareData: (initial, newerSnapshotFile, olderSnapshotFile) => {
    userStore.setState((state) => {
      const initRoot = {
        id: initial.id,
        name: initial.name,
        size: initial.meta.size,
        path: initial.path ?? initial.name,
        numsubdir: initial.meta.num_subdir,
        numsubfiles: initial.meta.num_files,
        children: [],
        childrenLoaded: false,
        scanDiscovered: true,
        diff: initial.meta.diff ? {
          new_flag: initial.meta.diff.new_dir_flag,
          deleted_flag: initial.meta.diff.deleted_dir_flag,
          prevnumsubdir: initial.meta.diff.prev_num_subdir,
          prevnumfiles: initial.meta.diff.prev_num_files,
          prevsize: initial.meta.diff.previous_size,
        } : undefined,
        directory: true,
      };

      return {
        analysisMode: "snapshot-compare",
        liveScanStatus: "idle",
        activeSnapshotFile: "",
        newerSnapshotFile,
        olderSnapshotFile,
        root: initRoot,
        currentEntryData: initRoot,
        currentPath: initRoot.path,
      };
    });
  },

  setSelectedHistorySnapshotFile: (fileName) => {
    set({ prevSnapshotFilePath: fileName })
  },

  setSnapshotFlag: (flag) => {
    set({ snapshotFlag: flag })
  }

}));
