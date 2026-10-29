<?php

namespace ComposerPatcher\Event;

use ComposerPatcher\Event;
use ComposerPatcher\Patch as PatchObject;

/**
 * Represent a patch-related event.
 */
class Patch extends Event
{
    /**
     * The patch.
     *
     * @var \ComposerPatcher\Patch
     */
    protected $patch;

    /**
     * Initialize the instance.
     *
     * @param string $name the event name
     */
    public function __construct($name, PatchObject $patch)
    {
        parent::__construct($name);
        $this->patch = $patch;
    }

    /**
     * Get the patch.
     *
     * @return \ComposerPatcher\Patch
     */
    public function getPatch()
    {
        return $this->patch;
    }
}
