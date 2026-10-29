<?php

namespace ComposerPatcher;

use Composer\Package\PackageInterface;

/**
 * The class that contains the collected patches.
 */
class PatchCollection
{
    /**
     * The list of collected patches.
     *
     * @var \ComposerPatcher\Patch[]
     */
    private $patches = array();

    /**
     * Add a collected patch.
     *
     * @return $this
     */
    public function addPatch(Patch $patch)
    {
        $this->patches[] = $patch;

        return $this;
    }

    /**
     * Get the list of collected patches.
     *
     * @return \ComposerPatcher\Patch[]
     */
    public function getPatches()
    {
        return $this->patches;
    }

    /**
     * Is this collection empty?
     *
     * @return bool
     */
    public function isEmpty()
    {
        return !isset($this->patches[0]);
    }

    /**
     * Merge another patch collection into this collection.
     *
     * @return $this
     */
    public function merge(self $otherCollection)
    {
        $this->patches = array_merge($this->patches, $otherCollection->getPatches());

        return $this;
    }

    /**
     * Check if this collection contains patches for a package.
     *
     * @param \Composer\Package\PackageInterface $package a PackageInterface instance
     *
     * @return bool
     */
    public function containsPatchesFor(PackageInterface $package)
    {
        foreach ($this->getPatches() as $patch) {
            if ($patch->isForPackage($package)) {
                return true;
            }
        }

        return false;
    }

    /**
     * Get the patches to be applied to a package.
     *
     * @param \Composer\Package\PackageInterface $package a PackageInterface instance
     *
     * @return static
     */
    public function getPatchesFor(PackageInterface $package)
    {
        $patches = array();
        foreach ($this->getPatches() as $patch) {
            if ($patch->isForPackage($package)) {
                $patches[] = $patch;
            }
        }
        $result = new static();
        $result->patches = $patches;

        return $result;
    }

    /**
     * Get the hash of all the patches.
     *
     * @return string
     */
    public function getHash()
    {
        $hashes = array();

        foreach ($this->getPatches() as $patch) {
            $hashes[] = $patch->getHash();
        }
        sort($hashes);

        return sha1(implode(' ', $hashes));
    }
}
