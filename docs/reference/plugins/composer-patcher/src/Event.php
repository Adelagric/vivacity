<?php

namespace ComposerPatcher;

use Composer\EventDispatcher\Event as ComposerEvent;

/**
 * A base class for the ComposerPatcher events.
 */
abstract class Event extends ComposerEvent
{
    /**
     * The name of the event fired right before applying a patch.
     *
     * @var string
     */
    const EVENTNAME_PRE_APPLY_PATCH = 'pre-apply-patch';

    /**
     * The name of the event fired right after a patch has been applied.
     *
     * @var string
     */
    const EVENTNAME_POST_APPLY_PATCH = 'post-apply-patch';
}
