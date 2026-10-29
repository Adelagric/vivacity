<?php

namespace ComposerPatcher;

use Composer\Composer;
use Composer\DependencyResolver\Operation\UninstallOperation;
use Composer\EventDispatcher\EventSubscriberInterface;
use Composer\IO\IOInterface;
use Composer\Package\AliasPackage;
use Composer\Package\PackageInterface;
use Composer\Package\RootPackageInterface;
use Composer\Plugin\PluginInterface;
use Composer\Script\Event as ComposerEvent;
use Composer\Script\ScriptEvents;
use Composer\Util\ProcessExecutor;
use Composer\Util\RemoteFilesystem;
use ComposerPatcher\Util\PathResolver;
use Exception as GenericException;

class Plugin implements PluginInterface, EventSubscriberInterface
{
    /**
     * @var \Composer\Composer
     */
    private $composer;

    /**
     * @var \Composer\IO\IOInterface
     */
    private $io;

    /**
     * @var \ComposerPatcher\PatchCollection|null
     */
    private $patchCollection;

    /**
     * @var \ComposerPatcher\Util\VolatileDirectory|null
     */
    private $volatileDirectory;

    /**
     * @var \Composer\Util\RemoteFilesystem|null
     */
    private $remoteFilesystem;

    /**
     * @var \ComposerPatcher\Util\PathResolver|null
     */
    private $pathResolver;

    /**
     * @var bool|null
     */
    private $considerPatchErrorsAsWarnings;

    /**
     * @var \ComposerPatcher\PatchCollector|null
     */
    private $patchCollector;

    /**
     * @var \ComposerPatcher\Patcher|null
     */
    private $patcher;

    /**
     * {@inheritdoc}
     *
     * @see \Composer\Plugin\PluginInterface::activate()
     */
    public function activate(Composer $composer, IOInterface $io)
    {
        $this->composer = $composer;
        $this->io = $io;
    }

    /**
     * {@inheritdoc}
     *
     * @see \Composer\Plugin\PluginInterface::deactivate()
     */
    public function deactivate(Composer $composer, IOInterface $io)
    {
    }

    /**
     * {@inheritdoc}
     *
     * @see \Composer\Plugin\PluginInterface::uninstall()
     */
    public function uninstall(Composer $composer, IOInterface $io)
    {
    }

    /**
     * {@inheritdoc}
     *
     * @see \Composer\EventDispatcher\EventSubscriberInterface::getSubscribedEvents()
     */
    public static function getSubscribedEvents()
    {
        return array(
            ScriptEvents::PRE_INSTALL_CMD => array('preOperation'),
            ScriptEvents::PRE_UPDATE_CMD => array('preOperation'),
            ScriptEvents::POST_INSTALL_CMD => array('postOperation'),
            ScriptEvents::POST_UPDATE_CMD => array('postOperation'),
        );
    }

    /**
     * Before running composer install/update, let's uninstall some packages.
     */
    public function preOperation(ComposerEvent $event)
    {
        $patchCollection = $this->getPatchCollection();
        if ($patchCollection->isEmpty()) {
            return;
        }
        $packages = $this->getLocalRepositoryPackages();
        $promises = array();
        foreach ($packages as $package) {
            if ($this->mustUninstallPackage($package)) {
                $promise = $this->uninstallPackage($package);
                if ($promise) {
                    $promises[] = $promise;
                }
            }
        }
        if ($promises !== array() && method_exists($this->composer, 'getLoop')) {
            $this->composer->getLoop()->wait($promises);
        }
    }

    /**
     * After running composer install/update, let's patch some packages.
     *
     * @throws \Exception
     */
    public function postOperation(ComposerEvent $event)
    {
        $this->refreshPatchCollection();
        $patchCollection = $this->getPatchCollection();
        if ($patchCollection->isEmpty()) {
            $this->io->write('<info>No patches supplied.</info>');

            return;
        }
        $packages = $this->getLocalRepositoryPackages();
        foreach ($packages as $package) {
            if ($this->mustInstallPatchesForPackage($package)) {
                $this->installPatchesForPackage($package, $event->isDevMode());
            }
        }
    }

    /**
     * @return string
     */
    protected function getPatchTemporaryFolder()
    {
        $result = '';
        $extra = $this->composer->getPackage()->getExtra();
        if (isset($extra['patch-temporary-folder'])) {
            $ptf = $extra['patch-temporary-folder'];
            if ($ptf !== null && $ptf !== '') {
                if (!\is_string($ptf)) {
                    $this->io->write("<error>The value of extra.patch-temporary-folder must be a string: we'll use the system temporary folder.</error>");
                } else {
                    $ptf = str_replace(\DIRECTORY_SEPARATOR, '/', $ptf);
                    if ($ptf !== '/') {
                        $ptf = rtrim($ptf, '/');
                    }
                    if (!is_dir($ptf)) {
                        $ptf = str_replace('/', \DIRECTORY_SEPARATOR, $ptf);
                        $this->io->write("<error>The value of extra.patch-temporary-folder '{$ptf}' does not exist: we\'ll use the system temporary folder.</error>");
                    } elseif (!is_writable($ptf)) {
                        $ptf = str_replace('/', \DIRECTORY_SEPARATOR, $ptf);
                        $this->io->write("<error>The value of extra.patch-temporary-folder '{$ptf}' is not writable: we\'ll use the system temporary folder.</error>");
                    } else {
                        $result = $ptf;
                    }
                }
            }
        }

        return $result;
    }

    /**
     * @return \ComposerPatcher\Util\VolatileDirectory
     */
    protected function getVolatileDirectory()
    {
        if ($this->volatileDirectory === null) {
            $this->volatileDirectory = new Util\VolatileDirectory($this->getPatchTemporaryFolder());
        }

        return $this->volatileDirectory;
    }

    /**
     * @return \Composer\Util\RemoteFilesystem
     */
    protected function getRemoteFilesystem()
    {
        if ($this->remoteFilesystem === null) {
            $this->remoteFilesystem = new RemoteFilesystem($this->io, $this->composer->getConfig());
        }

        return $this->remoteFilesystem;
    }

    /**
     * @return \ComposerPatcher\Util\PathResolver
     */
    protected function getPathResolver()
    {
        if ($this->pathResolver === null) {
            $this->pathResolver = new PathResolver($this->getVolatileDirectory(), $this->getRemoteFilesystem());
        }

        return $this->pathResolver;
    }

    /**
     * @return bool
     */
    protected function considerPatchErrorsAsWarnings()
    {
        if ($this->considerPatchErrorsAsWarnings === null) {
            $extra = $this->composer->getPackage()->getExtra();
            if (isset($extra['patch-errors-as-warnings'])) {
                $this->considerPatchErrorsAsWarnings = (bool) $extra['patch-errors-as-warnings'];
            } else {
                $this->considerPatchErrorsAsWarnings = true;
            }
        }

        return $this->considerPatchErrorsAsWarnings;
    }

    /**
     * @return \ComposerPatcher\PatchCollector
     */
    protected function getPatchCollector()
    {
        if ($this->patchCollector === null) {
            $this->patchCollector = new PatchCollector($this->getPathResolver(), $this->io, $this->composer->getInstallationManager(), $this->considerPatchErrorsAsWarnings());
        }

        return $this->patchCollector;
    }

    /**
     * The list of patches to be applied.
     *
     * @return \ComposerPatcher\PatchCollection
     */
    protected function getPatchCollection()
    {
        if ($this->patchCollection === null) {
            $patchCollector = $this->getPatchCollector();
            $packages = $this->getLocalRepositoryPackages(true);
            $this->patchCollection = $patchCollector->collectPatches($this->composer->getPackage(), $packages);
        }

        return $this->patchCollection;
    }

    /**
     * Reset the patch collection, so that the next call to getPatchCollection() will return fresh data.
     */
    protected function refreshPatchCollection()
    {
        $this->patchCollection = null;
    }

    /**
     * Get the Patcher instance.
     *
     * @return \ComposerPatcher\Patcher
     */
    protected function getPatcher()
    {
        if ($this->patcher === null) {
            $this->patcher = new Patcher($this->io, $this->composer->getInstallationManager(), $this->composer->getEventDispatcher(), new ProcessExecutor($this->io), $this->getVolatileDirectory());
        }

        return $this->patcher;
    }

    /**
     * Check if a package must be uninstalled before the install/update commands.
     *
     * @param \Composer\Package\PackageInterface $package the package to be checked
     *
     * @return bool
     */
    protected function mustUninstallPackage(PackageInterface $package)
    {
        if ($package instanceof AliasPackage) {
            return false;
        }
        $packageExtra = $package->getExtra();
        $packageHasAppliedPatches = !empty($packageExtra['patches_applied']);
        $packagePatches = $this->getPatchCollection()->getPatchesFor($package);
        $packageHasPatches = !$packagePatches->isEmpty();
        if ($packageHasAppliedPatches === false && $packageHasPatches === false) {
            return false;
        }
        if ($packageHasAppliedPatches === true && $packageHasPatches === true) {
            $installedHash = empty($packageExtra['patches_applied']['hash']) ? '' : $packageExtra['patches_applied']['hash'];
            if ($installedHash === '') {
                return true;
            }
            $patchesHash = $packagePatches->getHash();
            if ($installedHash !== $patchesHash) {
                return true;
            }
        }

        return false;
    }

    /**
     * Should we apply patches to the package?
     *
     * @param \Composer\Package\PackageInterface $package the package to be patched
     *
     * @return bool
     */
    protected function mustInstallPatchesForPackage(PackageInterface $package)
    {
        $result = false;
        $patches = $this->getPatchCollection()->getPatchesFor($package);
        if ($patches->isEmpty()) {
            if ($this->io->isVerbose()) {
                $this->io->write('<info>No patches found for '.$package->getName().'.</info>');
            }
        } else {
            $result = true;
        }

        return $result;
    }

    /**
     * Install the patches for a specific package.
     *
     * @param \Composer\Package\PackageInterface $package the package to be patched
     * @param bool $isDevMode
     */
    protected function installPatchesForPackage(PackageInterface $package, $isDevMode)
    {
        $considerPatchErrorsAsWarnings = $this->considerPatchErrorsAsWarnings();
        $patches = $this->getPatchCollection()->getPatchesFor($package);
        $patcher = $this->getPatcher();
        $appliedPatchesToSave = method_exists($package, 'setExtra') ? array() : null;
        foreach ($patches->getPatches() as $patch) {
            try {
                $patcher->applyPatch($patch, $package);
                if ($appliedPatchesToSave !== null) {
                    $appliedPatchesToSave[] = array(
                        'from-package' => $patch->getFromPackage()->getPrettyString(),
                        'path' => $patch->getOriginalPath(),
                        'description' => $patch->getDescription(),
                    );
                }
            } catch (GenericException $x) {
                $appliedPatchesToSave = null;
                if (!$considerPatchErrorsAsWarnings) {
                    throw $x;
                }
                $this->io->write('<error>'.$x->getMessage().'</error>');
            }
        }
        if ($appliedPatchesToSave !== null) {
            $this->setPatchedPackageData($package, array(
                'hash' => $patches->getHash(),
                'list' => $appliedPatchesToSave,
            ), $isDevMode);
        }
    }

    /**
     * Persist the patches_applied to the package extra section.
     *
     * @param bool $isDevMode
     */
    protected function setPatchedPackageData(PackageInterface $package, array $patchesAppliedData, $isDevMode)
    {
        $extra = $package->getExtra();
        $extra['patches_applied'] = $patchesAppliedData;
        $package->setExtra($extra);
        if (\defined('Composer\Composer::RUNTIME_API_VERSION') && version_compare(Composer::RUNTIME_API_VERSION, '2') >= 0) {
            $this->composer->getRepositoryManager()->getLocalRepository()->write($isDevMode, $this->composer->getInstallationManager());
        } else {
            $this->composer->getRepositoryManager()->getLocalRepository()->write();
        }
    }

    /**
     * Uninstall a package.
     *
     * @param \Composer\Package\PackageInterface $package the package to be uninstalled
     *
     * @return \React\Promise\PromiseInterface|null
     */
    protected function uninstallPackage(PackageInterface $package)
    {
        $uninstallOperation = new UninstallOperation($package, 'Removing package so it can be re-installed and re-patched.');
        $this->io->write('<info>Removing package '.$package->getName().' so that it can be re-installed and re-patched.</info>');
        $installationManager = $this->composer->getInstallationManager();
        $repositoryManager = $this->composer->getRepositoryManager();
        $localRepository = $repositoryManager->getLocalRepository();

        return $installationManager->uninstall($localRepository, $uninstallOperation);
    }

    /**
     * Get the packages from the local Composer repository.
     *
     * @param bool $excludeRootPackage
     *
     * @return \Composer\Package\PackageInterface[]
     */
    protected function getLocalRepositoryPackages($excludeRootPackage = false)
    {
        $repositoryManager = $this->composer->getRepositoryManager();
        $localRepository = $repositoryManager->getLocalRepository();
        $packages = $localRepository->getPackages();
        $rootRepository = null;
        $result = array();
        foreach ($packages as $package) {
            if ($package instanceof RootPackageInterface) {
                $rootRepository = $package;
                if (!$excludeRootPackage) {
                    $result[] = $package;
                }
            } else {
                $result[] = $package;
            }
        }
        if (!$excludeRootPackage && $rootRepository === null) {
            $result[] = $this->composer->getPackage();
        }

        return $result;
    }
}
