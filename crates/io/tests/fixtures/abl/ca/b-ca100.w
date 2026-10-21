/* ca/b-ca100.w: browser part of the ca100 screen. */
ASSIGN vfolder = "Liste|Compte".
RUN set-attribute-list IN h_folder ('FOLDER-LABELS = ':U + vfolder + ',').
