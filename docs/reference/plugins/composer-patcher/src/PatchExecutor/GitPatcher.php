<?php

namespace ComposerPatcher\PatchExecutor;

use Composer\IO\IOInterface;
use Composer\Util\ProcessExecutor;
use ComposerPatcher\Exception;
use ComposerPatcher\Patch;
use ComposerPatcher\PatchExecutor;
use ComposerPatcher\Util\VolatileDirectory;

class GitPatcher extends PatchExecutor
{
    /**
     * @var \ComposerPatcher\PatchExecutor\PatchPatcher
     */
    private $patchPatcher;

    /**
     * Initialize the instance.
     *
     * @param \Composer\Util\ProcessExecutor $processExecutor the ProcessExecutor instance to be used to actually run commands
     * @param \Composer\IO\IOInterface $io the IOInterface instance to be used for user feedback
     * @param \ComposerPatcher\Util\VolatileDirectory $volatileDirectory the VolatileDirectory instance to use to store temporary files
     *
     * @throws \ComposerPatcher\Exception\CommandNotFound when the git command is not available
     */
    public function __construct(ProcessExecutor $processExecutor, IOInterface $io, VolatileDirectory $volatileDirectory)
    {
        parent::__construct($processExecutor, $io);
        $this->checkCommandExists('git');
        $this->patchPatcher = new PatchPatcher($processExecutor, $io, $volatileDirectory);
    }

    /**
     * Check if this PatchExecutor class should be used to apply patches to a directory.
     *
     * @param string $baseDirectory
     *
     * @return bool
     */
    public static function shouldBeUsetToApplyPatchesTo($baseDirectory)
    {
        return is_dir("{$baseDirectory}/.git");
    }

    /**
     * {@inheritdoc}
     *
     * @see \ComposerPatcher\PatchExecutor::applyPatchLevel()
     */
    protected function applyPatchLevel(Patch $patch, $baseDirectory, $patchLevel)
    {
        if ($this->patchPatcher->patchIsAlreadyApplied($patch, $baseDirectory, $patchLevel)) {
            throw new Exception\PatchAlreadyApplied($patch);
        }
        foreach (array(true, false) as $justTesting) {
            $command = $this->buildCommand($patch->getLocalPath(), $baseDirectory, $patchLevel, $justTesting);
            if ($justTesting && $this->io->isVerbose()) {
                $this->io->write("<comment>Testing ability to patch with \"git apply\" using patch level {$patchLevel} with the following command:\n{$command}</comment>");
            }
            list($rc, , $stdErr) = $this->run($command);
            if (0 !== $rc) {
                throw new Exception\PatchNotApplied($patch, "failed to apply the patch with GIT: {$stdErr}");
            }
            if (0 === strpos($stdErr, 'Skipped')) {
                return;
            }
        }
    }

    /**
     * @param string $localPatchFile
     * @param string $baseDirectory
     * @param string $patchLevel
     * @param bool $justTesting
     *
     * @return string
     */
    private function buildCommand($localPatchFile, $baseDirectory, $patchLevel, $justTesting)
    {
        $chunks = array(
            'git',
            // Run git as if it was started in $baseDirectory
            '-C ', $this->escape(str_replace('/', \DIRECTORY_SEPARATOR, $baseDirectory)),
            'apply',
            $this->escape($patchLevel),
        );
        if ($justTesting) {
            $chunks[] = '--check';
            $chunks[] = '-v';
        }
        $chunks[] = $this->escape(str_replace('/', \DIRECTORY_SEPARATOR, $localPatchFile));

        return implode(' ', $chunks);
    }
}
