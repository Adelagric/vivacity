<?php

namespace ComposerPatcher\Util;

use Composer\Util\Filesystem;
use ComposerPatcher\Exception;
use Exception as GenericException;

/**
 * A helper class that creates a temporary directory, and deletes it when the class is no more in use.
 */
class VolatileDirectory
{
    /**
     * @var \Composer\Util\Filesystem
     */
    private $filesystem;

    /**
     * @var string
     */
    private $parentDirectory;

    /**
     * A progressive counter used to build unique paths.
     *
     * @var int
     */
    private $uniqueCounter = 0;

    /**
     * The path to the volatile directory.
     *
     * @var string|null
     */
    private $path;

    /**
     * Initialize the instance.
     *
     * @param string $parentDirectory the parent directory where the volatile directory will be created
     * @param \Composer\Util\Filesystem $filesystem|null The composer file system instance to be used
     */
    public function __construct($parentDirectory = '', $filesystem = null)
    {
        $parentDirectory = str_replace(\DIRECTORY_SEPARATOR, '/', (string) $parentDirectory);
        if ($parentDirectory !== '/') {
            $parentDirectory = rtrim($parentDirectory, '/');
        }
        $this->parentDirectory = $parentDirectory;
        $this->filesystem = $filesystem instanceof Filesystem ? $filesystem : new Filesystem();
    }

    /**
     * Destroys the volatile directory.
     */
    public function __destruct()
    {
        if ($this->path !== null) {
            set_error_handler(function ($code, $msg) {
            });
            try {
                @$this->filesystem->removeDirectory($this->path);
            } catch (GenericException $x) {
            }
            restore_error_handler();
            $this->path = null;
        }
    }

    /**
     * Get the path of the system temporary directory.
     *
     * @throws \ComposerPatcher\Exception\PathNotFound when the temporary directory can't be retrieved or when it does not exist
     *
     * @return string
     */
    public static function getSystemTemporaryDirectory()
    {
        $tempDir = @sys_get_temp_dir();
        if (\is_string($tempDir)) {
            $tempDir = str_replace(\DIRECTORY_SEPARATOR, '/', (string) $tempDir);
            if ($tempDir !== '/') {
                $tempDir = rtrim($tempDir, '/');
            }
        } else {
            $tempDir = '';
        }
        if ($tempDir === '' || !is_dir($tempDir)) {
            throw new Exception\PathNotFound($tempDir, 'Unable to detect the system temporary directory.');
        }

        return $tempDir;
    }

    /**
     * Get the path of a new item inside this directory.
     *
     * @param string $suffix
     *
     * @return string
     */
    public function getNewPath($suffix = '')
    {
        $result = $this->getPath().'/'.$this->uniqueCounter.(string) $suffix;
        $this->uniqueCounter++;

        return $result;
    }

    /**
     * @return \Composer\Util\Filesystem
     */
    public function getFilesystem()
    {
        return $this->filesystem;
    }

    /**
     * Get the path to the volatile directory.
     *
     * @throws \ComposerPatcher\Exception\PathNotFound when the parent directory is not specified and the system temporary directory can't be retrieved or when it does not exist
     * @throws \ComposerPatcher\Exception\PathNotFound when $parentDirectory is supplied but it does not exist
     * @throws \ComposerPatcher\Exception\PathNotWritable when the directory where the volatile directory will be created is not writable
     * @throws \ComposerPatcher\Exception\PathNotCreated when the temporary directory could not be created
     *
     * @return string
     */
    public function getPath()
    {
        if ($this->path === null) {
            $parentDirectory = $this->parentDirectory;
            if ($parentDirectory === '') {
                $parentDirectory = static::getSystemTemporaryDirectory();
            } else {
                if (!is_dir($parentDirectory)) {
                    throw new Exception\PathNotFound($parentDirectory);
                }
            }
            if (!is_writable($parentDirectory)) {
                throw new Exception\PathNotWritable($parentDirectory);
            }
            while (true) {
                $tempNam = @tempnam($parentDirectory, 'PCH');
                if ($tempNam === false) {
                    throw new Exception\PathNotCreated("{$parentDirectory}/<TEMPORARY>");
                }
                @unlink($tempNam);
                if (@mkdir($tempNam)) {
                    break;
                }
            }
            $this->path = rtrim(str_replace(\DIRECTORY_SEPARATOR, '/', $tempNam), '/');
        }

        return $this->path;
    }
}
