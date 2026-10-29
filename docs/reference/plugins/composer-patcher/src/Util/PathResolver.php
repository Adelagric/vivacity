<?php

namespace ComposerPatcher\Util;

use Composer\Util\RemoteFilesystem;
use ComposerPatcher\Exception;

/**
 * A helper class that resolves a single path.
 */
class PathResolver
{
    /**
     * The temporary directory that will hold downloaded patches.
     *
     * @var \ComposerPatcher\Util\VolatileDirectory
     */
    private $volatileDirectory;

    /**
     * The Filesystem instance to be used to work with local files.
     *
     * @var \Composer\Util\Filesystem
     */
    private $filesystem;

    /**
     * The RemoteFilesystem instance to be used to work with remote files.
     *
     * @var \Composer\Util\RemoteFilesystem
     */
    private $remoteFilesystem;

    /**
     * A list of already resolved remote paths.
     *
     * @var array
     */
    private $resolvedRemotePath = array();

    /**
     * Initialize the instance.
     *
     * @param \ComposerPatcher\Util\VolatileDirectory $volatileDirectory the temporary directory that will hold downloaded patches
     * @param \Composer\Util\RemoteFilesystem $remoteFilesystem the RemoteFilesystem instance to be used to work with remote files
     */
    public function __construct(VolatileDirectory $volatileDirectory, RemoteFilesystem $remoteFilesystem)
    {
        $this->volatileDirectory = $volatileDirectory;
        $this->remoteFilesystem = $remoteFilesystem;
        $this->filesystem = $volatileDirectory->getFilesystem();
    }

    /**
     * Resolve a local or remote path.
     *
     * @param string $path The path to be resolved
     * @param string $baseFolder the absolute path to be used when $path is a local relative path
     *
     * @throws \ComposerPatcher\Exception\PathNotFound when $path is not found
     *
     * @return string Empty string if $path is not a string or if it's an empty string
     */
    public function resolve($path, $baseFolder)
    {
        if (!\is_string($path)) {
            return '';
        }
        if (stripos($path, 'file://') === 0) {
            $path = substr($path, 7);
        }
        if ($path === '') {
            return '';
        }
        if (preg_match('_^\w\w+://_', $path)) {
            return $this->resolveRemotePath($path);
        }

        return $this->resolveLocalPath($path, $baseFolder);
    }

    /**
     * Resolve a local path.
     *
     * @param string $path The path to be resolved
     * @param string $baseFolder the absolute path to be used when $path is a local relative path
     *
     * @throws \ComposerPatcher\Exception\PathNotFound when $path is not found
     *
     * @return string
     */
    private function resolveLocalPath($path, $baseFolder)
    {
        $path = $this->filesystem->normalizePath($path);
        if ($this->filesystem->isAbsolutePath($path)) {
            return $path;
        }
        if (stripos($baseFolder, 'file://') === 0) {
            $baseFolder = substr($baseFolder, 7);
        }
        $baseFolder = $this->filesystem->normalizePath($baseFolder);

        return $baseFolder.'/'.$path;
    }

    /**
     * Resolve a local path.
     *
     * @param string $path The path to be resolved
     *
     * @throws \ComposerPatcher\Exception\PathNotFound when $path is not found
     *
     * @return string
     */
    private function resolveRemotePath($path)
    {
        if (!isset($this->resolvedRemotePath[$path])) {
            $resolvedPath = $this->volatileDirectory->getNewPath();
            if ($this->remoteFilesystem->copy($path, $path, $resolvedPath) !== true) {
                throw new Exception\PathNotFound($path);
            }
            $this->resolvedRemotePath[$path] = $resolvedPath;
        }

        return $this->resolvedRemotePath[$path];
    }
}
