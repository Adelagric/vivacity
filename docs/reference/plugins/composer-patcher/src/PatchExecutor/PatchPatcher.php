<?php

namespace ComposerPatcher\PatchExecutor;

use Composer\IO\IOInterface;
use Composer\Util\ProcessExecutor;
use ComposerPatcher\Exception;
use ComposerPatcher\Patch;
use ComposerPatcher\PatchExecutor;
use ComposerPatcher\Util\VolatileDirectory;

class PatchPatcher extends PatchExecutor
{
    /**
     * The actual patch command to be executed.
     *
     * @var string
     */
    private $command;

    /**
     * The VolatileDirectory instance to use to store temporary files.
     *
     * @var \ComposerPatcher\Util\VolatileDirectory
     */
    private $volatileDirectory;

    /**
     * @var bool|null
     */
    private $useNoBackupIfMismatch;

    /**
     * Initialize the instance.
     *
     * @param \Composer\Util\ProcessExecutor $processExecutor the ProcessExecutor instance to be used to actually run commands
     * @param \Composer\IO\IOInterface $io the IOInterface instance to be used for user feedback
     * @param \ComposerPatcher\Util\VolatileDirectory $volatileDirectory the VolatileDirectory instance to use to store temporary files
     *
     * @throws \ComposerPatcher\Exception\CommandNotFound when the patch command is not available
     */
    public function __construct(ProcessExecutor $processExecutor, IOInterface $io, VolatileDirectory $volatileDirectory)
    {
        parent::__construct($processExecutor, $io);
        $this->volatileDirectory = $volatileDirectory;
        if (\DIRECTORY_SEPARATOR === '\\') {
            $this->initializeWindows();
        } else {
            $this->initializePosix();
        }
    }

    /**
     * Check if a patch is already applied.
     *
     * @param \ComposerPatcher\Patch $patch the patch to be analyzed
     * @param string $baseDirectory the directory where the patch should be applied
     * @param string $patchLevel the patch level
     *
     * @return bool
     */
    public function patchIsAlreadyApplied(Patch $patch, $baseDirectory, $patchLevel)
    {
        $command = $this->buildCommand($patch->getLocalPath(), $baseDirectory, $patchLevel, true);
        list($rc, $stdOut, $stdErr) = $this->run($command);
        if ($stdErr === '') {
            $stdErr = $stdOut;
        }
        if ($rc === 1 && strpos($stdErr, 'Skipping patch.') !== false && preg_match('/^(\\d+) out of \\1 hunks? ignored/m', $stdErr)) {
            return true;
        }

        return false;
    }

    /**
     * {@inheritdoc}
     *
     * @see \ComposerPatcher\PatchExecutor::applyPatchLevel()
     */
    protected function applyPatchLevel(Patch $patch, $baseDirectory, $patchLevel)
    {
        if ($this->patchIsAlreadyApplied($patch, $baseDirectory, $patchLevel)) {
            throw new Exception\PatchAlreadyApplied($patch);
        }
        $command = $this->buildCommand($patch->getLocalPath(), $baseDirectory, $patchLevel, false);
        list($rc, $stdOut, $stdErr) = $this->run($command);
        if ($rc === 0) {
            return;
        }
        if ($stdErr === '') {
            $stdErr = $stdOut;
        }
        throw new Exception\PatchNotApplied($patch, "failed to apply the patch with the patch command: {$stdErr}");
    }

    /**
     * Configure the instance for POSIX.
     *
     * @throws \ComposerPatcher\Exception\CommandNotFound when the patch command is not available
     */
    private function initializePosix()
    {
        $this->checkCommandExists('patch');
        $this->command = 'patch';
    }

    /**
     * Configure the instance for Windows.
     *
     * @throws \ComposerPatcher\Exception\CommandNotFound when the patch command is not available
     */
    private function initializeWindows()
    {
        $gitFolder = $this->findWindowsGitFolder();
        if ($gitFolder !== '') {
            $patchPath = "{$gitFolder}\\usr\\bin\\patch.exe";
            if (is_file($patchPath)) {
                try {
                    $patchPath = escapeshellarg($patchPath);
                    $this->checkCommandExists($patchPath);
                    $this->command = $patchPath;

                    return;
                } catch (Exception\CommandNotFound $x) {
                }
            }
        }
        try {
            $this->checkCommandExists('patch');
            $this->command = 'patch';

            return;
        } catch (Exception\CommandNotFound $x) {
        }
        throw new Exception\CommandNotFound('patch', <<<'EOT'
The patch command is currently not available.
You have these options:
1. Install Git for Windows, and add it to the PATH environment variable
2. Find a Windows port of the "patch" GNU utility and it to a directory in the current PATH
EOT
        );
    }

    /**
     * Search the path where git is installed on Windows.
     *
     * @return string Empty string if not found
     */
    private function findWindowsGitFolder()
    {
        list($rc, $stdOut) = $this->run('where git.exe');
        if ($rc !== 0) {
            return '';
        }
        $stdOut = str_replace("\r\n", "\n", $stdOut);
        foreach (explode("\n", trim($stdOut)) as $path) {
            $easierPath = str_replace('\\', '/', $path);
            $match = null;
            if (preg_match('_^(.+)/(?:cmd|bin)/git\.exe$_i', $easierPath, $match)) {
                return str_replace('/', '\\', $match[1]);
            }
        }

        return '';
    }

    /**
     * @param string $localPatchFile
     * @param string $baseDirectory
     * @param string $patchLevel
     * @param bool $dryRun
     *
     * @return string
     */
    private function buildCommand($localPatchFile, $baseDirectory, $patchLevel, $dryRun)
    {
        $chunks = array(
            $this->command,
            $this->escape($patchLevel),
            // Back up mismatches only if otherwise requested
            $this->useNoBackupIfMismatch() ? '--no-backup-if-mismatch' : '',
            // Ignore patches where the differences have already been applied to the file (aka --forward)
            '-N',
            // Change the working directory (aka --directory)
            '-d', $this->escape(str_replace('/', \DIRECTORY_SEPARATOR, $baseDirectory)),
            // Read patch from PATCHFILE instead of stdin (aka --input)
            '-i', $this->escape(str_replace('/', \DIRECTORY_SEPARATOR, $localPatchFile)),
            // Output rejects to FILE (aka --reject-file)
            '-r', $this->escape(str_replace('/', \DIRECTORY_SEPARATOR, $this->volatileDirectory->getNewPath('.rej'))),
        );
        if ($dryRun) {
            // Do not actually change any files; just print what would happen.
            $chunks[] = '--dry-run';
        }
        $chunks = array_filter($chunks, function ($chunk) { return $chunk !== ''; });

        return implode(' ', $chunks);
    }

    /**
     * Should we use --no-backup-if-mismatch?
     * --no-backup-if-mismatch is required with the GNU implementation of patch in order to avoid backups in case if mismatches.
     * On FreeBSD that's the default behaviour, and the --no-backup-if-mismatch is not available.
     *
     * @return bool
     */
    private function useNoBackupIfMismatch()
    {
        if ($this->useNoBackupIfMismatch === null) {
            list(, $stdOut, $stdErr) = $this->run("{$this->command} --help");
            // On GNU we have the usage in stdOut
            // On FreeBSD the --help option does not exists, and the usage is sent to stdErr
            $this->useNoBackupIfMismatch = strpos((string) $stdOut, '--no-backup-if-mismatch') !== false || strpos((string) $stdErr, '--no-backup-if-mismatch') !== false;
        }

        return $this->useNoBackupIfMismatch;
    }
}
