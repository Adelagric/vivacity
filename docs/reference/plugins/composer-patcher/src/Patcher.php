<?php

namespace ComposerPatcher;

use Composer\EventDispatcher\EventDispatcher;
use Composer\Installer\InstallationManager;
use Composer\IO\IOInterface;
use Composer\Package\PackageInterface;
use Composer\Package\RootPackage;
use Composer\Util\ProcessExecutor;
use ComposerPatcher\Event\Patch as PatchEvent;
use ComposerPatcher\PatchExecutor\GitPatcher;
use ComposerPatcher\PatchExecutor\PatchPatcher;
use ComposerPatcher\Util\VolatileDirectory;

class Patcher
{
    /**
     * @var \Composer\IO\IOInterface
     */
    private $io;

    /**
     * The InstallationManager instance to be used to resolve package installation paths.
     *
     * @var \Composer\Installer\InstallationManager
     */
    private $installationManager;

    /**
     * @var \Composer\EventDispatcher\EventDispatcher
     */
    private $eventDispatcher;

    /**
     * @var \Composer\Util\ProcessExecutor
     */
    private $processExecutor;

    /**
     * @var \ComposerPatcher\PatchExecutor\GitPatcher|null
     */
    private $gitPatcher;

    /**
     * @var \ComposerPatcher\PatchExecutor\PatchPatcher|null
     */
    private $patchPatcher;

    /**
     * @var \ComposerPatcher\Util\VolatileDirectory
     */
    private $volatileDirectory;

    public function __construct(IOInterface $io, InstallationManager $installationManager, EventDispatcher $eventDispatcher, ProcessExecutor $processExecutor, VolatileDirectory $volatileDirectory)
    {
        $this->io = $io;
        $this->installationManager = $installationManager;
        $this->eventDispatcher = $eventDispatcher;
        $this->processExecutor = $processExecutor;
        $this->volatileDirectory = $volatileDirectory;
    }

    /**
     * Apply a patch to a package.
     *
     * @throws \Exception in case of errors
     */
    public function applyPatch(Patch $patch, PackageInterface $package)
    {
        $this->io->write('<info>Applying patch '.$patch->getFromPackage()->getName().'/'.$patch->getDescription().' to '.$package->getName().'... </info>', false);
        if ($package instanceof RootPackage) {
            $baseDirectory = getcwd();
        } else {
            $baseDirectory = $this->installationManager->getInstallPath($package);
        }
        $this->eventDispatcher->dispatch(null, new PatchEvent(PatchEvent::EVENTNAME_PRE_APPLY_PATCH, $patch));
        try {
            if (GitPatcher::shouldBeUsetToApplyPatchesTo($baseDirectory)) {
                $this->getGitPatcher()->applyPatch($patch, $baseDirectory);
            } else {
                $this->getPatchPatcher()->applyPatch($patch, $baseDirectory);
            }
            $this->io->write('<info>done.</info>');
        } catch (Exception\PatchAlreadyApplied $x) {
            $this->io->write('<info>patch was already applied.</info>');
        }
        $this->eventDispatcher->dispatch(null, new PatchEvent(PatchEvent::EVENTNAME_POST_APPLY_PATCH, $patch));
    }

    /**
     * @throws \ComposerPatcher\Exception\CommandNotFound when the git command is not available
     *
     * @return \ComposerPatcher\PatchExecutor\GitPatcher
     */
    protected function getGitPatcher()
    {
        if (null === $this->gitPatcher) {
            $this->gitPatcher = new GitPatcher($this->processExecutor, $this->io, $this->volatileDirectory);
        }

        return $this->gitPatcher;
    }

    /**
     * @throws \ComposerPatcher\Exception\CommandNotFound when the git command is not available
     *
     * @return \ComposerPatcher\PatchExecutor\PatchPatcher
     */
    protected function getPatchPatcher()
    {
        if (null === $this->patchPatcher) {
            $this->patchPatcher = new PatchPatcher($this->processExecutor, $this->io, $this->volatileDirectory);
        }

        return $this->patchPatcher;
    }
}
