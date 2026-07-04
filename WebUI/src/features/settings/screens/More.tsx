/*
 * Copyright (C) Contributors to the Suwayomi project
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

import { Fragment } from 'react';
import { useTranslation } from 'react-i18next';
import ListAltIcon from '@mui/icons-material/ListAlt';
import List from '@mui/material/List';
import ListItemIcon from '@mui/material/ListItemIcon';
import ListItemText from '@mui/material/ListItemText';
import Divider from '@mui/material/Divider';
import { AppRoutes } from '@/base/AppRoute.constants.ts';
import { ListItemLink } from '@/base/components/lists/ListItemLink.tsx';
import { NAVIGATION_BAR_ITEMS } from '@/features/navigation-bar/NavigationBar.constants.ts';
import { MediaQuery } from '@/base/utils/MediaQuery.tsx';
import { NavigationBarUtil } from '@/features/navigation-bar/NavigationBar.util.ts';
import { useMetadataServerSettings } from '@/features/settings/services/ServerSettingsMetadata.ts';
import { useAppTitle } from '@/features/navigation-bar/hooks/useAppTitle.ts';
import { useNavigationSettings } from '@/features/navigation-bar/NavigationBar.hooks.ts';
import type { NavbarItem } from '@/features/navigation-bar/NavigationBar.types.ts';

export const More = () => {
    const { t } = useTranslation();
    const isMobileWidth = MediaQuery.useIsMobileWidth();

    useAppTitle(t('global.label.more'));

    const {
        settings: { hideHistory },
    } = useMetadataServerSettings();
    const { visibleTabs } = useNavigationSettings();

    const hiddenNavBarItems = NavigationBarUtil.getHiddenItems(NAVIGATION_BAR_ITEMS, {
        hideHistory,
        hideBoth: false,
        hideDesktop: isMobileWidth,
        hideMobile: !isMobileWidth,
        visibleTabs,
    });

    const hiddenItemMoreGroup = NAVIGATION_BAR_ITEMS.find((item) => item.path === AppRoutes.downloads.path)?.moreGroup;

    if (hiddenItemMoreGroup == null) {
        throw new Error('Unable to find hidden navigation item group');
    }

    const hiddenNavBarItemsByMoreGroup = hiddenNavBarItems.reduce<
        Partial<Record<NavbarItem['moreGroup'], NavbarItem[]>>
    >(
        (groups, item) => ({
            ...groups,
            [item.moreGroup]: [...(groups[item.moreGroup] ?? []), item],
        }),
        {},
    );

    const hiddenItemsMoreGroup = [
        ...(hiddenNavBarItemsByMoreGroup[hiddenItemMoreGroup] ?? []),
        {
            path: AppRoutes.settings.childRoutes.categories.path,
            title: 'category.title.category_other',
            SelectedIconComponent: ListAltIcon,
            IconComponent: ListAltIcon,
            show: 'both',
            moreGroup: hiddenItemMoreGroup,
        },
    ] satisfies NavbarItem[];

    const finalHiddenNavBarItemsByGroup: typeof hiddenNavBarItemsByMoreGroup = {
        ...hiddenNavBarItemsByMoreGroup,
        [hiddenItemMoreGroup]: hiddenItemsMoreGroup,
    };

    return (
        <List sx={{ p: 0 }}>
            {Object.entries(finalHiddenNavBarItemsByGroup).map(([group, items], index, list) => (
                <Fragment key={group}>
                    {items.map((item) => (
                        <Fragment key={item.path}>
                            <ListItemLink key={item.path} to={item.path}>
                                <ListItemIcon>
                                    <item.IconComponent />
                                </ListItemIcon>
                                <ListItemText
                                    primary={t(item.moreTitle ?? item.title)}
                                    secondary={item.useBadge?.().title}
                                />
                            </ListItemLink>
                        </Fragment>
                    ))}
                    {index !== list.length - 1 && <Divider />}
                </Fragment>
            ))}
        </List>
    );
};
